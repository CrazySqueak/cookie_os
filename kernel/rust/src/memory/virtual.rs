
use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod arch;

#[path = "paging_firstfit.rs"]
mod impl_firstfit;

pub trait PageFrameAllocator {
    const NPAGES: usize;
    const PAGE_SIZE: usize;
    type PageTableType: IPageTable;
    
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
    
    /* Attempt to allocate the requested amount of memory. */
    fn allocate(&mut self, size: usize) -> Option<()>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<()>;
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
    unsafe fn alloc_subtable(&mut self, idx: usize, ptr: *const u8);
    
    unsafe fn alloc_subtable_from_allocator<PFA: PageFrameAllocator>(&mut self, idx: usize, allocator: &PFA){
        self.alloc_subtable(idx, allocator.get_page_table_ptr() as *const u8)
    }
}

pub trait PageAllocation {
    // TODO
}