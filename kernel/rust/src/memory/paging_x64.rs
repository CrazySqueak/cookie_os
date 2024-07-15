
use x86_64::structures::paging::page_table::{PageTable,PageTableEntry};

use super::*;
use super::impl_firstfit::MLFFAllocator;

impl IPageTable for PageTable {
    const NPAGES: usize = 512;
    
    fn new() -> Self {
        Self::new()
    }
    
    fn is_unused(&self, idx: usize) -> bool {
        self[idx].is_unused()
    }
    
    fn get_num_pages_used(&self) -> usize {
        self.iter().filter(|e| e.is_unused()).count()
    }
}

type X64_LEVEL_1 = MLFFAllocator<    ()     , PageTable, false, true >;  // Page Table
type X64_LEVEL_2 = MLFFAllocator<X64_LEVEL_1, PageTable, true , true >;  // Page Directory
type X64_LEVEL_3 = MLFFAllocator<X64_LEVEL_2, PageTable, true , false>;  // Page Directory Pointer Table
type X64_LEVEL_4 = MLFFAllocator<X64_LEVEL_3, PageTable, true , false>;  // Page Map Level 4