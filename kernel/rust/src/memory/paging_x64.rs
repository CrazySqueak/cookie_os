
use x86_64::structures::paging::page_table::{PageTable,PageTableEntry,PageTableFlags};
use x86_64::addr::PhysAddr;

use crate::logging::klog;

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
    
    // alloc
    unsafe fn alloc_huge(&mut self, idx: usize){
        let flags = match LEVEL {
            1 => PageTableFlags::empty(),  // (huge page flag is used for PAT on level 1 page tables)
            _ => PageTableFlags::HUGE_PAGE,
        };
        
        self.0[idx].set_addr(PhysAddr::new(0), flags); 
    }
    unsafe fn alloc_subtable(&mut self, idx: usize, phys_addr: usize){
        let flags = PageTableFlags::WRITABLE | PageTableFlags::PRESENT;  // set these two by default in case the page gets subdivided, and also set PRESENT because the page table is literally right fucking there
        klog!(Debug, "memory.paging.map", "Mapping sub-table {:x}[{}] -> {:x}", ptaddr_virt_to_phys(core::ptr::addr_of!(self.0) as usize), idx, phys_addr);
        self.0[idx].set_addr(PhysAddr::new(phys_addr as u64), flags);
        // TODO
    }
    
    // modification
    unsafe fn set_addr(&mut self, idx: usize, physaddr: usize){
        let flags = self.0[idx].flags() | PageTableFlags::PRESENT | PageTableFlags::WRITABLE;  // set PRESENT flag by default (i mean, where else is the page gonna be if you're setting it to physical memory?) TODO: also add a way to configure flags or something. the whole "allocation.set_x" system is very messy currently
        klog!(Debug, "memory.paging.map", "Mapping entry {:x}[{}] to {:x}", ptaddr_virt_to_phys(core::ptr::addr_of!(self.0) as usize), idx, physaddr);
        self.0[idx].set_addr(PhysAddr::new(physaddr as u64), flags);
    }
    
    // cr3
    unsafe fn activate(&self){
        assert_eq!(LEVEL, 4, "Cannot activate a page table of this level!");
        use core::ptr::addr_of;
        use x86_64::addr::PhysAddr;
        use x86_64::structures::paging::frame::PhysFrame;
        use x86_64::registers::control::Cr3;
        
        let phys_addr = ptaddr_virt_to_phys(addr_of!(self.0) as usize);
        let (_, cr3flags) = Cr3::read();
        klog!(Info, "memory.paging", "Activating page addr=0x{:x} cr3flags={:?}", phys_addr, cr3flags);
        Cr3::write(PhysFrame::from_start_address(PhysAddr::new(phys_addr.try_into().unwrap())).expect("Page Table Address Not Aligned!"), cr3flags)
    }
}

type X64Level1 = MLFFAllocator<NoDeeper , X64PageTable<1>, false, true >;  // Page Table
type X64Level2 = MLFFAllocator<X64Level1, X64PageTable<2>, true , true >;  // Page Directory

#[cfg(not(feature="1G_huge_pages"))]
type X64Level3 = MLFFAllocator<X64Level2, X64PageTable<3>, true , false>;  // Page Directory Pointer Table
#[cfg(feature="1G_huge_pages")]
type X64Level3 = MLFFAllocator<X64Level2, X64PageTable<3>, true , true>;  // Page Directory Pointer Table

type X64Level4 = MLFFAllocator<X64Level3, X64PageTable<4>, true , false>;  // Page Map Level 4

pub type TopLevelPageTable = X64Level4;
/* Discard the upper 16 bits of an address (for 48-bit vmem) */
pub fn crop_addr(addr: usize) -> usize {
    addr & 0x0000_ffff_ffff_ffff
}
/* Convert a virtual address to a physical address, for use with pointing the CPU to page tables. */
pub fn ptaddr_virt_to_phys(vaddr: usize) -> usize {
    vaddr-crate::lowlevel::HIGHER_HALF_OFFSET // note: this will break if the area where the page table lives is not offset-mapped (or if the address has been cropped to hold all 0s for non-canonical bits)
}