
use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod implementation;

trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    
    /* Create a new, empty page frame allocator. */
    fn new() -> Self;
    /* Get the number of pages which are occupied. */
    fn get_num_pages_used(&self) -> usize;
    
    /* Attempt to allocate the requested amount of memory. */
    fn allocate(&mut self, size: usize) -> Option<()>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<()>;
}


