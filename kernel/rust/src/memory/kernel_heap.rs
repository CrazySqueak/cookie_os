
use core::alloc::Layout;
use buddy_system_allocator::LockedHeap;

// use crate::lowlevel::_without_interrupts;
// 
use crate::logging::klog;

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
        // _without_interrupts(||{
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
        // })
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // _without_interrupts(||{
            self.heap.dealloc(ptr, layout)
        // })
    }
}

#[global_allocator]
static KHEAP_ALLOCATOR: KernelHeap = KernelHeap::new();

pub unsafe fn init_kheap(){
    // Init heap
    KHEAP_ALLOCATOR.init(kheap_initial_addr as usize,kheap_initial_size as usize);
    
    // Success
    klog!(Info, MEMORY_KHEAP, "Initialised kernel heap with {} bytes.", kheap_initial_size);
}

pub unsafe fn init_kheap_2(){
    // Init rescue
    // _reinit_rescue::spawn();
}

unsafe fn on_oom(heap: &LockedHeap<32>, layout: &Layout) {
    todo!()
}

/*
/* Allocate some extra physical memory to the kernel heap.
    (note: the kernel heap's PhysicalMemoryAllocations are currently not kept anywhere, so they cannot be freed again. Use this function with care.)
    Size is in bytes.
    Return value is the actual number of bytes added, or Err if it was unable to allocate the requested space.
    */
pub fn grow_kheap(amount: usize) -> Result<usize,()>{
    use super::physical::palloc;
    use core::mem::forget;
    use core::alloc::Layout;
    
    // Step 1: allocate physical memory
    let l = Layout::from_size_align(amount, 4096).unwrap();
    klog!(Debug, MEMORY_KHEAP, "Expanding kernel heap by {} bytes...", amount);
    let allocation = palloc(l).ok_or_else(||{
        klog!(Severe, MEMORY_KHEAP, "Unable to expand kernel heap! Requested {} bytes but allocation failed.", amount);
    })?;
    let bytes_added = allocation.get_size();
    
    // Step 2: allocate virtual memory
    use super::paging::global_pages::KERNEL_PTABLE;
    use super::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
    let valloc = KERNEL_PTABLE.allocate_at(KERNEL_PTABLE.get_vmem_offset()+allocation.get_addr(), bytes_added).ok_or_else(||{
        klog!(Severe, MEMORY_KHEAP, "Unable to expand kernel heap! Received memory that is unable to be mapped?");  // This should only occur if somehow the page mappings have been fucked up (or i've added swapping support but didn't update this)
    })?;
    valloc.set_base_addr(allocation.get_addr(), PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::empty()));
    
    klog!(Info, MEMORY_KHEAP, "Expanded kernel heap by {} bytes.", bytes_added);
    
    unsafe {
        // Add allocation to heap
        KHEAP_ALLOCATOR.init(valloc.start(), bytes_added);
        // forget the allocation
        // (so that the destructor isn't called)
        // (as the destructor frees it)
        forget(allocation); forget(valloc);
    }
    
    Ok(bytes_added)
}

// RESCUE
use super::paging::global_pages::KERNEL_PTABLE;
use super::paging::PageAllocation;
use super::physical::{palloc,PhysicalMemoryAllocation};
use buddy_system_allocator::Heap;
use core::alloc::Layout;
use crate::sync::kspin::KMutex;  // use a spinlock for the heap rather than scheduler yield - the scheduler could easily end up using the heap
// As allocating new memory may require heap memory, we keep a 1MiB rescue section pre-allocated.
const RESCUE_SIZE: usize = 1*1024*1024;  // 1MiB
type RescueT = (PhysicalMemoryAllocation,PageAllocation<super::paging::global_pages::GlobalPTType>);
static GLOBAL_RESCUE: KMutex<Option<RescueT>> = KMutex::new(None);

fn on_oom(heap: &LockedHeap<32>, layout: &Layout){
    unsafe {
        if _use_rescue(heap).is_ok() {
            // Reinit rescue if we successfully used the previous one
            // (we can't re-init if rescue failed because there might not be enough heap space to allocate more memory)
            // (which is the reason why we have a pre-allocated rescue in the first place)
            //_reinit_rescue(&mut rescue);
            _reinit_rescue::spawn();
        }
    }
}

// note: interrupts are already disabled when _use_rescue is called
unsafe fn _use_rescue(heap: &LockedHeap<32>) -> Result<(),()> {
    // Expand heap
    let (pallocation, vallocation) = match GLOBAL_RESCUE.try_lock().and_then(|mut r|r.take()){ Some(x)=>x, None=>return Err(()), };
    heap.lock().init(vallocation.start(), vallocation.size());  // (note: this assumes that the allocation is contiguous (which it should be))
    // Forget allocation so that it doesn't get Drop'd and deallocated
    core::mem::forget(pallocation); vallocation.leak();
    // klog!(Info, MEMORY_KHEAP, "Rescued kernel heap.");
    Ok(())
}

use crate::multitasking::util::def_task_fn;
def_task_fn!(task fn _reinit_rescue(){
    klog!(Debug, MEMORY_KHEAP, "Allocating new rescue...");
    let kernel_table = &KERNEL_PTABLE;
    let newrescue = (||{
        use crate::memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
        let pallocation = palloc(Layout::from_size_align(RESCUE_SIZE,1).unwrap())?;
        let vallocation = kernel_table.allocate_at(KERNEL_PTABLE.get_vmem_offset()+pallocation.get_addr(), pallocation.get_size())?;
        vallocation.set_base_addr(pallocation.get_addr(), PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::empty()));
        Some((pallocation, vallocation))
    })();
    match newrescue {
        Some(nr) => {
            let paddr = nr.0.get_addr(); let vaddr = nr.1.start(); let vsize = nr.1.size();
            crate::multitasking::without_interruptions(||{let _=GLOBAL_RESCUE.lock().insert(nr);});
            klog!(Info, MEMORY_KHEAP, "Allocated new rescue @ V={:x} size {} | P={:x}", vaddr, vsize, paddr);
        },
        None => {
            klog!(Severe, MEMORY_KHEAP, "Unable to allocate new rescue! Next kernel OOM will crash!");
        },
    }
});
*/