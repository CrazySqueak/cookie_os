
use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod implementation;

trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    
    /* Create a new, empty page frame allocator. */
    fn new() -> Self;
    
    /* Attempt to allocate the requested amount of memory. */
    fn allocate(&mut self, size: usize) -> Option<()>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<()>;
}


