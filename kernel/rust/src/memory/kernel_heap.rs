
use buddy_system_allocator::LockedHeap;

use crate::logging::klog;

extern "C" {
    // Provided by longmode.intel.asm (64-bit)
    pub static kheap_initial_addr: usize;
    pub static kheap_initial_size: usize;
}

#[global_allocator]
static KHEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();

pub fn init_kheap(){
    unsafe {
        KHEAP_ALLOCATOR.lock().init(kheap_initial_addr as usize,kheap_initial_size as usize);
    }
}

/* Allocate some extra physical memory to the kernel heap.
    (note: the kernel heap's PhysicalMemoryAllocations are currently not kept anywhere, so they cannot be freed again. Use this function with care.)
    Size is in bytes.
    Return value is the actual number of bytes added, or Err if it was unable to allocate the requested space.
    Note: The Kernel cannot reference memory outside of the first 1G unless you update the page table mappings. TODO: Replace this system with something more robust (necessary anyway once I add paging and multitasking)
    */
pub fn grow_kheap(amount: usize) -> Result<usize,()>{
    use super::physical::palloc;
    use core::mem::forget;
    use core::alloc::Layout;
    
    // TODO: Paging support
    let l = Layout::from_size_align(amount, 4096).unwrap();
    klog!(Debug, MEMORY_KHEAP, "Expanding kernel heap by {} bytes...", amount);
    let allocation = palloc(l).ok_or_else(||{
        klog!(Severe, MEMORY_KHEAP, "Unable to expand kernel heap! Requested {} bytes but allocation failed.", amount);
    })?;
    
    let bytes_added = allocation.get_size();
    klog!(Info, MEMORY_KHEAP, "Expanded kernel heap by {} bytes.", bytes_added);
    
    crate::lowlevel::without_interrupts(||{ unsafe {
        // Add allocation to heap
        KHEAP_ALLOCATOR.lock().init((allocation.get_ptr().as_ptr() as usize) + crate::lowlevel::HIGHER_HALF_OFFSET, bytes_added);
        // forget the allocation
        // (so that the destructor isn't called)
        // (as the destructor frees it)
        forget(allocation);
    }});
    
    Ok(bytes_added)
}