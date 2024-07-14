
use alloc::vec::Vec;
use alloc::boxed::Box;

use crate::logging::klog;

trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    
    /* Attempt to allocate the requested amount of memory. */
    fn create_allocation(&mut self, size: usize) -> ();
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn create_allocation_at(&mut self, addr: usize, size: usize) -> ();
}

struct X64PageAllocator<ST, const SUBTABLES: bool, const HUGEPAGES: bool> {
    page_table: (), // TODO
    
    suballocators: [Option<Box<ST>>; 512],
    // Each addr is 2 bits: 00 = empty, 01 = occupied by table, 10 = occupied by table (half full), 11 = full / occupied by page
    availability_bitmap: [u8; 128],
}
impl<ST, const SUBTABLES: bool, const HUGEPAGES: bool> PageFrameAllocator for X64PageAllocator<ST,SUBTABLES,HUGEPAGES>
    where ST: PageFrameAllocator {
    const NPAGES: usize = 512;
    const PAGE_SIZE: usize = if SUBTABLES { ST::PAGE_SIZE * ST::NPAGES } else { 4096 };
    
    // TODO
}

type X64_LEVEL_1 = X64PageAllocator<    ()     , false, true >;  // Page Table
type X64_LEVEL_2 = X64PageAllocator<X64_LEVEL_1, true , true >;  // Page Directory
type X64_LEVEL_3 = X64PageAllocator<X64_LEVEL_2, true , false>;  // Page Directory Pointer Table
type X64_LEVEL_4 = X64PageAllocator<X64_LEVEL_3, true , false>; // Page Map Level 4

