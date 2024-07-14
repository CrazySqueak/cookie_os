
use alloc::vec::Vec;
use alloc::boxed::Box;

use crate::logging::klog;

trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    
    /* Attempt to allocate the requested amount of memory. */
    fn allocate(&mut self, size: usize) -> Option<()>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<()>;
}

struct X64PageAllocator<ST, const SUBTABLES: bool, const HUGEPAGES: bool> {
    page_table: (), // TODO
    
    suballocators: [Option<Box<ST>>; 512],
    // Each addr is 2 bits: 00 = empty, 01 = occupied by table, 10 = occupied by table (half full), 11 = full / occupied by page
    availability_bitmap: [u8; 128],
}
enum _AllocAt {
    Start,
    End,
    None,
}
impl<ST, const SUBTABLES: bool, const HUGEPAGES: bool> PageFrameAllocator for X64PageAllocator<ST,SUBTABLES,HUGEPAGES>
    where ST: PageFrameAllocator {
    const NPAGES: usize = 512;
    const PAGE_SIZE: usize = if SUBTABLES { ST::PAGE_SIZE * ST::NPAGES } else { 4096 };
    
    fn allocate(&mut self, size: usize) -> Option<()> {
        // We only support a non-page-sized remainder if we support sub-tables (as page frames cannot be divided)
        let pages = if SUBTABLES { size / Self::NPAGES } else { size.div_ceil(Self::NPAGES) };
        let remainder = if SUBTABLES { size % Self::NPAGES } else { 0 };
        
        // TODO: Replace with something that's not effectively O(n^2)
        for offset in 0..(Self::NPAGES-pages) {
            'check: {
                let start = offset; let end = offset+pages;
                
                // Test contiguous middle section
                for i in start..end { if self.availability_bitmap[i] != 0b000u8 { break 'check; } };
                
                // Test remainder (if applicable)
                if remainder != 0 && SUBTABLES {
                    let required_availability: u8 = if remainder >= Self::NPAGES/2 { 0b001u8 } else { 0b010u8 };  // Only test half-full ones if our remainder is small enough
                    if start > 0 && self.availability_bitmap[start-1] <= required_availability {
                        // ???
                    } else if end < Self::NPAGES && self.availability_bitmap[end] <= required_availability {
                        // ???
                    } else {
                        // cannot allocate remainder
                        break 'check
                    }
                }
                
                // All tests were successful. IDK what to do here
            }
        }
        todo!();
    }
}

type X64_LEVEL_1 = X64PageAllocator<    ()     , false, true >;  // Page Table
type X64_LEVEL_2 = X64PageAllocator<X64_LEVEL_1, true , true >;  // Page Directory
type X64_LEVEL_3 = X64PageAllocator<X64_LEVEL_2, true , false>;  // Page Directory Pointer Table
type X64_LEVEL_4 = X64PageAllocator<X64_LEVEL_3, true , false>; // Page Map Level 4

