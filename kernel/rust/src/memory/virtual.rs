
use alloc::vec::Vec;

use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod arch;
pub use arch::TopLevelPageTable;

#[path = "paging_firstfit.rs"]
mod impl_firstfit;
#[path = "paging_nodeeper.rs"]
mod impl_nodeeper;
use impl_nodeeper::NoDeeper;

pub trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    type PageTableType: IPageTable;
    type SubAllocType: PageFrameAllocator;
    
    /* Create a new, empty page frame allocator. */
    fn new() -> Self;
    /* Get the number of pages which are occupied. */
    fn get_num_pages_used(&self) -> usize;
    
    /* Get a pointer to this allocator's page table.
       (used for pointing higher-level page tables to their children) */
    fn get_page_table_ptr(&self) -> *const Self::PageTableType;
    /* Get a mutable reference to this allocator's page table.
        (used for modifying the table post-allocation in a manner that is compatible with Rust's mutability rules) */
    fn get_page_table_mut(&mut self) -> &mut Self::PageTableType;
    /* Get a mutable reference to the given sub-allocator, or None if unsupported/not present. */
    fn get_suballocator_mut(&mut self, index: usize) -> Option<&mut Self::SubAllocType>;
    
    /* Attempt to allocate the requested amount of memory. */
    fn allocate(&mut self, size: usize) -> Option<PageAllocation>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PageAllocation>;
}

pub trait IPageTable {
    const NPAGES: usize;
    
    /* Creates a new, empty page table. */ 
    fn new() -> Self;
    /* Returns true if the specified page is unused (e.g. zeroed out on x64), false otherwise. */
    fn is_unused(&self, idx: usize) -> bool;
    /* Get the number of pages currenty used */
    fn get_num_pages_used(&self) -> usize;
    
    // SAFETY: Modifying page tables is prone to cause UB if done incorrectly
    /* Allocate a full page, and ????? */
    unsafe fn alloc_huge(&mut self, idx: usize);
    /* Allocate a sub-page-table, and return ????? */
    unsafe fn alloc_subtable(&mut self, idx: usize, phys_addr: usize);
    
    unsafe fn alloc_subtable_from_allocator<PFA: PageFrameAllocator>(&mut self, idx: usize, allocator: &PFA){
        self.alloc_subtable(idx, (allocator.get_page_table_ptr() as usize)-crate::lowlevel::HIGHER_HALF_OFFSET)  // note: this will break if the area where the page table lives is not offset-mapped
    }
    
    /* Set the address for the given item (huge pages only, not subtables). */
    unsafe fn set_addr(&mut self, idx: usize, physaddr: usize);
}

#[derive(Debug)]
pub struct PAllocEntry{index: usize, offset: usize}
#[derive(Debug)]
pub struct PAllocSubAlloc{index: usize, offset: usize, alloc: PageAllocation}
/* NOTE: PageAllocation is equivalent to an index into an array. If you drop it without deallocating it, you will leak VMem (as well as actual memory keeping the page tables in RAM).*/
#[derive(Debug)]
pub struct PageAllocation {
    pagetableaddr: *const u8,
    entries: Vec<PAllocEntry>,  
    suballocs: Vec<PAllocSubAlloc>,  
    // (offset is the offset for the start of the frame/subpage in physmem, measured from the base physmem address)
}
impl PageAllocation {
    fn new(pagetableaddr: *const u8, entries: Vec<PAllocEntry>, suballocs: Vec<PAllocSubAlloc>) -> Self {
        Self {
            pagetableaddr, entries, suballocs,
        }
    }
    
    /* For some reason, PageAllocators are not stored in a multi-owner type such as Arc<>, so instead we have this jank to ensure
        you have a mutable reference to the page table before you can edit the allocation's flags. */
    pub fn modify<'a,'b,PFA>(&'b self, allocator: &'a mut PFA) -> PageAllocationMut<'a,'b,PFA>
        where PFA: PageFrameAllocator {
        assert_eq!(self.pagetableaddr, allocator.get_page_table_ptr() as *const u8);
        PageAllocationMut {
            allocator: allocator,
            allocation: self,
        }
    }
}
#[derive(Debug)]
pub struct PageAllocationMut<'a,'b, PFA: PageFrameAllocator> {
    allocator: &'a mut PFA,
    allocation: &'b PageAllocation,
}
impl<PFA:PageFrameAllocator> PageAllocationMut<'_,'_,PFA> {
    /* Set the base physical address for this allocation.
        (this will then page to [baseaddr,baseaddr+size) */
    pub unsafe fn set_base_addr(&mut self, baseaddr: usize){
        let pagetable = self.allocator.get_page_table_mut();
        for PAllocEntry{index, offset} in &self.allocation.entries {
            pagetable.set_addr(*index, baseaddr+offset);
        }
        
        for PAllocSubAlloc{index, offset, alloc} in &self.allocation.suballocs {
            let allocator = self.allocator.get_suballocator_mut(*index).expect("Allocated allocator not found!");
            let mut alloc_mut = alloc.modify(allocator);
            alloc_mut.set_base_addr(baseaddr+offset);
        }
    }
}