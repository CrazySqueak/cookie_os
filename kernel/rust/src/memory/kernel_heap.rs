
use buddy_system_allocator::LockedHeap;

use crate::logging::klog;

extern "C" {
    // Provided by longmode.intel.asm (64-bit)
    pub static kheap_initial_addr: usize;
    pub static kheap_initial_size: usize;
}

struct KernelHeap {
    heap: LockedHeap<32>,
    rescue: Mutex<Option<RescueT>>,
}
impl KernelHeap {
    pub const fn new() -> Self {
        Self { heap: LockedHeap::new(), rescue: Mutex::new(None), }
    }
    
    pub unsafe fn init(&self, addr: usize, size: usize){
        self.heap.lock().init(addr, size)
    }
}
unsafe impl core::alloc::GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // We MUST not be interrupted while the heap is locked
        crate::lowlevel::without_interrupts(||{
            let result = self.heap.lock().alloc(layout);
            match result {
                Ok(result)=>result.as_ptr(),
                Err(_) => {
                    // Attempt rescue
                    on_oom(&self.heap, &layout, &self.rescue);
                    // Attempt re-allocation
                    self.heap.alloc(layout)
                }
            }
        })
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.heap.dealloc(ptr, layout)
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
    let mut rescue = KHEAP_ALLOCATOR.rescue.lock();
    _reinit_rescue(&mut rescue);
}

/*/* Allocate some extra physical memory to the kernel heap.
//    (note: the kernel heap's PhysicalMemoryAllocations are currently not kept anywhere, so they cannot be freed again. Use this function with care.)
//    Size is in bytes.
//    Return value is the actual number of bytes added, or Err if it was unable to allocate the requested space.
//    Note: The Kernel cannot reference memory outside of the first 1G unless you update the page table mappings. TODO: Replace this system with something more robust (necessary anyway once I add paging and multitasking)
//    */
//pub fn grow_kheap(amount: usize) -> Result<usize,()>{
//    use super::physical::palloc;
//    use core::mem::forget;
//    use core::alloc::Layout;
//    
//    // TODO: Paging support
//    let l = Layout::from_size_align(amount, 4096).unwrap();
//    klog!(Debug, MEMORY_KHEAP, "Expanding kernel heap by {} bytes...", amount);
//    let allocation = palloc(l).ok_or_else(||{
//        klog!(Severe, MEMORY_KHEAP, "Unable to expand kernel heap! Requested {} bytes but allocation failed.", amount);
//    })?;
//    
//    let bytes_added = allocation.get_size();
//    klog!(Info, MEMORY_KHEAP, "Expanded kernel heap by {} bytes.", bytes_added);
//    
//    crate::lowlevel::without_interrupts(||{ unsafe {
//        // Add allocation to heap
//        KHEAP_ALLOCATOR.lock().init((allocation.get_ptr().as_ptr() as usize) + crate::lowlevel::HIGHER_HALF_OFFSET, bytes_added);
//        // forget the allocation
//        // (so that the destructor isn't called)
//        // (as the destructor frees it)
//        forget(allocation);
//    }});
//    
//    Ok(bytes_added)
//}*/

// RESCUE
use super::paging::global_pages::KERNEL_PTABLE;
use super::paging::PageAllocation;
use super::physical::{palloc,PhysicalMemoryAllocation};
use buddy_system_allocator::Heap;
use core::alloc::Layout;
use crate::sync::Mutex;
// As allocating new memory may require heap memory, we keep a 1MiB rescue section pre-allocated.
const RESCUE_SIZE: usize = 1*1024*1024;  // 1MiB
type RescueT = (PhysicalMemoryAllocation,PageAllocation);

fn on_oom(heap: &LockedHeap<32>, layout: &Layout, rescue: &Mutex<Option<RescueT>>){
    unsafe {
        let mut rescue = rescue.lock();
        if _use_rescue(heap, &mut rescue).is_ok() {
            // Reinit rescue if we successfully used the previous one
            // (we can't re-init if rescue failed because there might not be enough heap space to allocate more memory)
            // (which is the reason why we have a pre-allocated rescue in the first place)
            _reinit_rescue(&mut rescue);
        }
    }
}

unsafe fn _use_rescue(heap: &LockedHeap<32>, rescue: &mut Option<RescueT>) -> Result<(),()> {
    // Expand heap
    let (pallocation, vallocation) = match rescue.take(){ Some(x)=>x, None=>return Err(()), };
    heap.lock().init(vallocation.start(), vallocation.size());  // (note: this assumes that the allocation is contiguous (which it should be))
    // Forget allocation so that it doesn't get Drop'd and deallocated
    core::mem::forget((pallocation, vallocation));
    klog!(Info, MEMORY_KHEAP, "Rescued kernel heap.");
    Ok(())
}

unsafe fn _reinit_rescue(rescue: &mut Option<RescueT>){
    klog!(Debug, MEMORY_KHEAP, "Allocating new rescue...");
    let mut kernel_table = KERNEL_PTABLE.write_when_active();
    let newrescue = (||{
        use super::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
        let pallocation = palloc(Layout::from_size_align(RESCUE_SIZE,1).unwrap())?;
        let vallocation = kernel_table.allocate_at(KERNEL_PTABLE.get_vmem_offset()+pallocation.get_addr(), pallocation.get_size())?;
        kernel_table.set_base_addr(&vallocation, pallocation.get_addr(), PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::empty()));
        Some((pallocation, vallocation))
    })();
    match newrescue {
        Some(nr) => {
            let paddr = nr.0.get_addr(); let vaddr = nr.1.start(); let vsize = nr.1.size();
            let _ = rescue.insert(nr);
            klog!(Debug, MEMORY_KHEAP, "Allocated new rescue @ V={:x} size {} | P={:x}", vaddr, vsize, paddr);
        },
        None => {
            klog!(Severe, MEMORY_KHEAP, "Unable to allocate new rescue! Next kernel OOM will crash!");
        },
    }
}