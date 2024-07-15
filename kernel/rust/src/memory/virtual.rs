
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

impl<ST, const SUBTABLES: bool, const HUGEPAGES: bool> X64PageAllocator<ST,SUBTABLES,HUGEPAGES>
  where ST: PageFrameAllocator {
    fn get_subtable_always(&mut self, idx: usize) -> &mut Box<ST> {
        if let Some(ref mut subtable) = self.suballocators[idx] {
            return subtable;
        } else {
            // ???
            todo!();
        }
    }
    fn refresh_availability(&mut self, idx: usize){
        todo!();
    }
    
    // allocates indexes [start, end)
    fn _alloc_contiguous(&mut self, start: usize, end: usize) -> () { // TODO: return type
        for idx in start..end {
            // Allocate huge pages or subtables depending on if it's necessary
            assert!(self.availability_bitmap[idx] == 0b000u8);
            if HUGEPAGES {
                // Huge pages
                todo!();
            } else if SUBTABLES {
                // Sub-tables
                let subtable = self.get_subtable_always(idx);
                if let Some(allocation) = subtable.allocate(Self::PAGE_SIZE) {
                    todo!();
                } else {
                    panic!("This should never happen! allocation failed but did not match availability_bitmap!");
                }
            } else {
                panic!("Cannot have both SUBTABLES and HUGEPAGES set to false!");
            }
            self.refresh_availability(idx);
        };
        ()
    }
    fn _alloc_rem(&mut self, idx: usize, inner_offset: usize, size: usize) -> Option<()> {  // TODO: Return type
        let required_availability: u8 = if size >= Self::NPAGES/2 { 0b001u8 } else { 0b010u8 };  // Only test half-full ones if our remainder is small enough
        if idx < self.availability_bitmap.len() && self.availability_bitmap[idx] <= required_availability {
            // allocate remainder
            if let Some(allocation) = self.get_subtable_always(idx).allocate_at(inner_offset, size) {
                self.refresh_availability(idx);
                todo!();
                return Some(());
            }
        };
        None
    }
}
impl<ST, const SUBTABLES: bool, const HUGEPAGES: bool> PageFrameAllocator for X64PageAllocator<ST,SUBTABLES,HUGEPAGES>
  where ST: PageFrameAllocator {
    const NPAGES: usize = 512;
    const PAGE_SIZE: usize = if SUBTABLES { ST::PAGE_SIZE * ST::NPAGES } else { 4096 };
    
    fn allocate(&mut self, size: usize) -> Option<()> {
        // We only support a non-page-sized remainder if we support sub-tables (as page frames cannot be divided)
        let pages = if SUBTABLES { size / Self::PAGE_SIZE } else { size.div_ceil(Self::PAGE_SIZE) };
        let remainder = if SUBTABLES { size % Self::PAGE_SIZE } else { 0 };
        
        // TODO: Replace with something that's not effectively O(n^2)
        for offset in 0..(Self::NPAGES-pages+1) {
            'check: {
                let start = offset; let end = offset+pages;
                
                // Test contiguous middle section
                for i in start..end { if self.availability_bitmap[i] != 0b000u8 { break 'check; } };
                
                // Test remainder (and if successful, we've got it!)
                'allocrem: {
                  if remainder != 0 && SUBTABLES {
                    if let Some(alloc) = self._alloc_rem(start.wrapping_sub(1), Self::PAGE_SIZE-remainder, remainder){
                        todo!();
                        break 'allocrem;
                    }
                    if let Some(alloc) = self._alloc_rem(end, 0, remainder){
                        todo!();
                        break 'allocrem;
                    }
                    // cannot allocate remainder
                    break 'check;
                  }
                  // No remainder (or subtables not possible so already rounded up)
                }
                
                // Allocate middle
                self._alloc_contiguous(start, end);
            };
        }
        todo!();
    }
    
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<()> {
        // We can't have remainders less than PAGE_SIZE if we don't support subtables, so we round down and add to the size to make up the difference.
        let start_idx = addr / Self::PAGE_SIZE;
        let (start_rem, size) = if SUBTABLES { (addr % Self::PAGE_SIZE, size) } else { (0, size + (addr % Self::PAGE_SIZE)) };
        let pages = if SUBTABLES { size / Self::PAGE_SIZE } else { size.div_ceil(Self::PAGE_SIZE) };
        let remainder = if SUBTABLES { size % Self::PAGE_SIZE } else { 0 };
        let end = start_idx+size;
        
        // Check that the main area is clear
        for i in start_idx..end {
            if self.availability_bitmap[i] != 0b000u8 { return None; }
        }
        // Check that the remainder is clear (if applicable)
        'allocrem: { if remainder != 0 && SUBTABLES {
                if let Some(alloc) = self._alloc_rem(end, 0, remainder){
                    todo!();
                    break 'allocrem;
                }
            return None;
        }}
        // Allocate main section
        self._alloc_contiguous(start_idx, end);
        
        Some(())
    }
}

type X64_LEVEL_1 = X64PageAllocator<    ()     , false, true >;  // Page Table
type X64_LEVEL_2 = X64PageAllocator<X64_LEVEL_1, true , true >;  // Page Directory
type X64_LEVEL_3 = X64PageAllocator<X64_LEVEL_2, true , false>;  // Page Directory Pointer Table
type X64_LEVEL_4 = X64PageAllocator<X64_LEVEL_3, true , false>; // Page Map Level 4

