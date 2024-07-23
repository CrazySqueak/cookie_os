
use alloc::vec::Vec;

use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod arch;

pub use arch::{crop_addr,ptaddr_virt_to_phys};

#[path = "paging_firstfit.rs"]
mod impl_firstfit;
#[path = "paging_nodeeper.rs"]
mod impl_nodeeper;
use impl_nodeeper::NoDeeper;
#[path = "paging_api.rs"]
mod api;
pub use api::*;

pub(in self) trait PageFrameAllocator {
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
    fn allocate(&mut self, size: usize) -> Option<PartialPageAllocation>;
    /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PartialPageAllocation>;
}

pub(in self) trait IPageTable {
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
        self.alloc_subtable(idx, ptaddr_virt_to_phys(allocator.get_page_table_ptr() as usize))
    }
    
    /* Set the address for the given item (huge pages only, not subtables). */
    unsafe fn set_addr(&mut self, idx: usize, physaddr: usize);
    /* Set the given item as absent, and clear its present flag. */
    unsafe fn set_absent(&mut self, idx: usize, data: usize);
}

struct PAllocEntry{index: usize, offset: usize}
struct PAllocSubAlloc{index: usize, offset: usize, alloc: PartialPageAllocation}
// PartialPageAllocation stores the indicies and offsets of page allocations internally while allocation is being done
struct PartialPageAllocation {
    entries: Vec<PAllocEntry>,  
    suballocs: Vec<PAllocSubAlloc>,  
    // (offset is the offset for the start of the frame/subpage in physmem, measured from the base physmem address)
}
impl PartialPageAllocation {
    fn new(entries: Vec<PAllocEntry>, suballocs: Vec<PAllocSubAlloc>) -> Self {
        Self {
            entries, suballocs,
        }
    }
}