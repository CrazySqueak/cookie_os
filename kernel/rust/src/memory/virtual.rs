
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
#[path = "global_pages.rs"]
mod global_pages;

pub(in self) trait PageFrameAllocatorImpl {
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
#[allow(private_bounds)]
pub trait PageFrameAllocator: PageFrameAllocatorImpl {}
#[allow(private_bounds)]
impl<T: PageFrameAllocatorImpl> PageFrameAllocator for T {}

pub(in self) trait IPageTableImpl {
    const NPAGES: usize;
    
    /* Creates a new, empty page table. */ 
    fn new() -> Self;
    /* Returns true if the specified page is unused (e.g. zeroed out on x64), false otherwise. */
    fn is_unused(&self, idx: usize) -> bool;
    /* Get the number of pages currenty used */
    fn get_num_pages_used(&self) -> usize;
    
    /* Reserve a page (which will later be filled with a proper allocation.) */
    fn reserve(&mut self, idx: usize){
        // (at some point i need to document these silly page codes)
        self.set_absent(idx, 0xFFFF_FFFE_0000)
    }
    
    /* Initialise a sub-table at the given index.
        SAFETY: phys_addr must be the physical address of a page table. The given page table must not be freed while its entry still exists in this page table. */
    unsafe fn set_subtable_addr(&mut self, idx: usize, phys_addr: usize);
    /* Initialise a subtable, converting the given allocator to its table's address and using that.
        SAFETY: The allocator MUST outlive its entry in this page table. */
    unsafe fn set_subtable_addr_from_allocator<PFA: PageFrameAllocator>(&mut self, idx: usize, allocator: &PFA){
        self.set_subtable_addr(idx, ptaddr_virt_to_phys(allocator.get_page_table_ptr() as usize))
    }
    /* Set the address for the given item (huge pages only, not subtables). */
    fn set_huge_addr(&mut self, idx: usize, physaddr: usize);
    /* Set the given item as absent, and clear its present flag. */
    fn set_absent(&mut self, idx: usize, data: usize);
}
#[allow(private_bounds)]
pub trait IPageTable: IPageTableImpl {}
#[allow(private_bounds)]
impl<T: IPageTableImpl> IPageTable for T {}

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