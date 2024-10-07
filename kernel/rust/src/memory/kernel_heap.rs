
use core::alloc::Layout;
use buddy_system_allocator::LockedHeap;

use crate::logging::klog;
use crate::memory::paging::{pageFlags, PageAlignedValue, PageAllocationSizeT};
use crate::memory::unified::OffsetMappedAllocation;
use crate::multitasking::interruptions::_without_interruptions_noalloc;

extern "C" {
    // Provided by longmode.intel.asm (64-bit)
    pub static kheap_initial_addr: usize;
    pub static kheap_initial_size: usize;
}

struct KernelHeap {
    heap: LockedHeap<32>,
}
impl KernelHeap {
    pub const fn new() -> Self {
        Self { heap: LockedHeap::new() }
    }
    
    pub unsafe fn init(&self, addr: usize, size: usize){
        self.heap.lock().init(addr, size)
    }
}
unsafe impl core::alloc::GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // We MUST not be interrupted while the heap is locked
        _without_interruptions_noalloc(||unsafe{  // Safety: We do not call disable_interruptions at any point during this closure. on_oom might but said behaviour must be thouroughly checked.
            let result = self.heap.lock().alloc(layout);
            match result {
                Ok(result)=>result.as_ptr(),
                Err(_) => {
                    // Attempt rescue
                    on_oom(&self.heap, &layout);
                    // Attempt re-allocation
                    self.heap.alloc(layout)
                }
            }
        })
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        _without_interruptions_noalloc(||unsafe{
            self.heap.dealloc(ptr, layout)
         })
    }
}

#[global_allocator]
static KHEAP_ALLOCATOR: KernelHeap = KernelHeap::new();

pub unsafe fn init_kheap(){
    // Init heap
    KHEAP_ALLOCATOR.init(kheap_initial_addr as usize,kheap_initial_size as usize);

    // Success
    // Note: Logging would be unsafe here as the CPU locals have not been initialised yet (including the CPU number, which is referenced by the logger)
    //klog!(Info, MEMORY_KHEAP, "Initialised kernel heap with {} bytes.", kheap_initial_size);
}

pub(super) unsafe fn reclaim_for_heap(start: *mut u8, end: *mut u8) {
    klog!(Debug, MEMORY_KHEAP, "Reclaiming memory at {:x}-{:x} for heap.", start as usize, end as usize);
    _without_interruptions_noalloc(||
        KHEAP_ALLOCATOR.heap.lock().add_to_heap(start as usize, end as usize)
    );
}

pub unsafe fn init_kheap_2(){
    // Init rescue
    _reinit_rescue::spawn();
}

unsafe fn on_oom(heap: &LockedHeap<32>, layout: &Layout) {
    // N.B. Calling disable_interruptions in this function is unsafe unless it can be proven that the previous no_interruptions state will be restored exactly as it was by the end of this function.
    // This means that using KMutexes is probably safe provided you DROP THE GUARD before the end of the function, but not much else can be guaranteed.

   unsafe {
       if _use_rescue(heap).is_ok() {
           // Reinit rescue if we successfully used the previous one
           // (we can't re-init if rescue failed because there might not be enough heap space to allocate more memory)
           // (which is the reason why we have a pre-allocated rescue in the first place)
           _reinit_rescue::spawn();

           // Also, allocate more memory if possible, so we don't have to rescue so often
           _expand_further_post_rescue::spawn();
       }
   }
}

/* Allocate some extra physical memory to the kernel heap.
    (note: the kernel heap's PhysicalMemoryAllocations are currently not kept anywhere, so they cannot be freed again. Use this function with care.)
    Size is in bytes.
    Return value is the actual number of bytes added, or Err if it was unable to allocate the requested space.
    */
pub fn grow_kheap(amount: PageAllocationSizeT) -> Result<usize,()>{
    use super::physical::palloc;
    use core::mem::forget;
    use core::alloc::Layout;

    // Allocate memory
    klog!(Debug, MEMORY_KHEAP, "Expanding kernel heap by {} bytes...", amount);
    let allocation = OffsetMappedAllocation::alloc_new(amount, pageFlags!(t:WRITEABLE)).ok_or_else(||{
        klog!(Severe, MEMORY_KHEAP, "Unable to expand kernel heap! Requested {} bytes but allocation failed.", amount);
    })?;
    let bytes_added = allocation.get_size().get();
    
    unsafe {
        // Add allocation to heap
        KHEAP_ALLOCATOR.init(allocation.get_virt_addr().get(), bytes_added);
        // forget the allocation
        // (so that the destructor isn't called)
        // (as the destructor frees it)
        forget(allocation);
    }
    
    Ok(bytes_added)
}

// RESCUE
// As allocating new memory may require heap memory, we keep a 1MiB rescue section pre-allocated.
const RESCUE_SIZE: PageAllocationSizeT = PageAllocationSizeT::new_const(1*1024*1024);  // 1MiB
const POST_RESCUE_EXPAND_SIZE: PageAllocationSizeT = PageAllocationSizeT::new_const(7*1024*1024);  // 7MiB
type RescueT = OffsetMappedAllocation;
static GLOBAL_RESCUE: KMutex<Option<RescueT>> = KMutex::new(None);

// note: interrupts are already disabled when _use_rescue is called
unsafe fn _use_rescue(heap: &LockedHeap<32>) -> Result<(),()> {
    // Expand heap
    let allocation = match GLOBAL_RESCUE.try_lock().and_then(|mut r|r.take()){ Some(x)=>x, None=>return Err(()), };
    heap.lock().init(allocation.get_virt_addr().get(), allocation.get_size().get());
    // Forget allocation so that it doesn't get Drop'd and deallocated
    allocation.leak();
    _notify_rescue_used::spawn();  // we can't log here anymore so we launch a task instead
    Ok(())
}

use crate::multitasking::util::def_task_fn;
use crate::sync::kspin::KMutex;

def_task_fn!(task fn _notify_rescue_used() {
    klog!(Info, MEMORY_KHEAP, "Rescued kernel heap.");
});
def_task_fn!(task fn _reinit_rescue(){
    klog!(Debug, MEMORY_KHEAP, "Allocating new rescue...");
    let newrescue = OffsetMappedAllocation::alloc_new(RESCUE_SIZE, pageFlags!(t:WRITEABLE));
    match newrescue {
        Some(nr) => {
            klog!(Info, MEMORY_KHEAP, "Allocated new rescue @ V={:x} size {} | P={:x}", nr.get_virt_addr(), nr.get_size(), nr.get_phys_addr());
            *GLOBAL_RESCUE.lock() = Some(nr);
        }
        None => {
            klog!(Severe,MEMORY_KHEAP, "Unable to allocate new rescue! Next kernel OOM will crash!");
            // TODO: Reschedule again later
        }
    }
});
def_task_fn!(task fn _expand_further_post_rescue(){
    // Expand the heap further post-rescue
    let _ = grow_kheap(POST_RESCUE_EXPAND_SIZE);
});
