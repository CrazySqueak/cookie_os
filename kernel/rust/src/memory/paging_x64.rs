
use x86_64::structures::paging::page_table::{PageTable,PageTableEntry,PageTableFlags};
use x86_64::addr::PhysAddr;

use super::*;
use super::impl_firstfit::MLFFAllocator;

#[repr(transparent)]
pub struct X64PageTable<const LEVEL: usize>(PageTable);
impl<const LEVEL: usize> IPageTable for X64PageTable<LEVEL> {
    const NPAGES: usize = 512;
    
    fn new() -> Self {
        X64PageTable(PageTable::new())
    }
    
    fn is_unused(&self, idx: usize) -> bool {
        self.0[idx].is_unused()
    }
    
    fn get_num_pages_used(&self) -> usize {
        self.0.iter().filter(|e| e.is_unused()).count()
    }
    
    unsafe fn alloc_huge(&mut self, idx: usize){
        let flags = match LEVEL {
            1 => PageTableFlags::empty(),  // (huge page flag is used for PAT on level 1 page tables)
            _ => PageTableFlags::HUGE_PAGE,
        };
        
        self.0[idx].set_addr(PhysAddr::new(0), flags); 
    }
    unsafe fn alloc_subtable(&mut self, idx: usize, phys_addr: usize){
        let flags = PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;  // set these two by default in case the page gets subdivided
        self.0[idx].set_addr(PhysAddr::new(phys_addr as u64), flags);
        // TODO
    }
    
    unsafe fn set_addr(&mut self, idx: usize, physaddr: usize){
        let flags = self.0[idx].flags();
        self.0[idx].set_addr(PhysAddr::new(physaddr as u64), flags);
    }
}

type X64Level1 = MLFFAllocator<NoDeeper , X64PageTable<1>, false, true >;  // Page Table
type X64Level2 = MLFFAllocator<X64Level1, X64PageTable<2>, true , true >;  // Page Directory
type X64Level3 = MLFFAllocator<X64Level2, X64PageTable<3>, true , false>;  // Page Directory Pointer Table
type X64Level4 = MLFFAllocator<X64Level3, X64PageTable<4>, true , false>;  // Page Map Level 4

pub type TopLevelPageTable = X64Level4;
/* Discard the upper 16 bits of an address (for 48-bit vmem) */
pub fn crop_addr(addr: usize) -> usize {
    addr & 0x0000_ffff_ffff_ffff
}