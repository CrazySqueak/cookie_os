
use alloc::vec::Vec;
use alloc::boxed::Box;

use x86_64::structures::paging::page_table::{PageTable,PageTableEntry};

use super::*;

struct X64PageAllocator<ST, const SUBTABLES: bool, const HUGEPAGES: bool> {
    page_table: PageTable,
    
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
            // Create new allocator
            let new_st = self.suballocators[idx].insert(Box::new(ST::new()));
            // TODO: Add to page table
            todo!();
            // And return
            new_st
        }
    }
    
    fn get_availability(&self, i: usize) -> u8 {
        // 1122_3344 5566_7788
        (self.availability_bitmap[i/4] >> (6-(2*(i%4)))) & 0b11u8
    }
    fn refresh_availability(&mut self, idx: usize){
        let availability = 
            if let Some(alloc) = &self.suballocators[idx] {  // sub-table
                assert!(SUBTABLES);  // let statements in this position are unstable so fuck me i guess
                let npages_used = alloc.get_num_pages_used();
                if npages_used >= ST::NPAGES {
                    0b11u8  // full
                } else if npages_used >= ST::NPAGES/2 {
                    0b10u8  // half full +
                } else {
                    // less than half full
                    0b01u8
                }
            } else {  // page or empty
                if self.page_table[idx].is_unused() {
                    0b00u8  // empty
                } else {
                    0b11u8  // occupied by huge page
                }
            }
        ;
        let lsh = 6-(2*(idx%4));
        self.availability_bitmap[idx/4] ^= 0b11u8 << lsh; // clear
        self.availability_bitmap[idx/4] |= availability << lsh;  // set
    }
    
    // allocates indexes [start, end)
    fn _alloc_contiguous(&mut self, start: usize, end: usize) -> () { // TODO: return type
        for idx in start..end {
            // Allocate huge pages or subtables depending on if it's necessary
            assert!(self.get_availability(idx) == 0b00u8);
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
        let required_availability: u8 = if size >= Self::NPAGES/2 { 0b01u8 } else { 0b10u8 };  // Only test half-full ones if our remainder is small enough
        if idx < self.availability_bitmap.len()*4 && self.get_availability(idx) <= required_availability {
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
    
    fn new() -> Self {
        Self {
            page_table: PageTable::new(),
            
            suballocators: [const{None}; 512],
            availability_bitmap: [0; 128],
        }
    }
    
    fn get_num_pages_used(&self) -> usize {
        self.page_table.iter().filter(|e| e.is_unused()).count()
    }
    
    fn allocate(&mut self, size: usize) -> Option<()> {
        // We only support a non-page-sized remainder if we support sub-tables (as page frames cannot be divided)
        let pages = if SUBTABLES { size / Self::PAGE_SIZE } else { size.div_ceil(Self::PAGE_SIZE) };
        let remainder = if SUBTABLES { size % Self::PAGE_SIZE } else { 0 };
        
        // TODO: Replace with something that's not effectively O(n^2)
        for offset in 0..(Self::NPAGES-pages+1) {
            'check: {
                let start = offset; let end = offset+pages;
                
                // Test contiguous middle section
                for i in start..end { if self.get_availability(i) != 0b00u8 { break 'check; } };
                
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
            if self.get_availability(i) != 0b00u8 { return None; }
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