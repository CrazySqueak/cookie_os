
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
pub mod global_pages;

mod sealed {
    use super::*;
    
    pub trait PageFrameAllocatorImpl {
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
        
        /* Add a reference to a global table. Panics if the index is already in use. Once the global table is added, it must not be overwritten or re-allocated.
            get_suballocator_mut must still return None for this index, as getting a mutable reference to a global table would violate Rust's aliasing rules.
            SAFETY: The given address must be the physical address of the table. Global page tables are expected to belong to the 'static lifetime.
                    Global page tables must be present for a given vmem addr in all paging contexts, as it is not cleared from the TLB when switching.
                    And many more. Here be dragons. */
        unsafe fn put_global_table(&mut self, index: usize, phys_addr: usize, flags: PageFlags);
    }

    pub trait IPageTableImpl {
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
        /* Add the given flags to the subtable. */
        fn add_subtable_flags<const INCLUDE_NON_TRANSITIVE: bool>(&mut self, idx: usize, flags: &PageFlags);  // (uses monomorphisation for optimisation or something)
        
        /* Set the address for the given item (huge pages only, not subtables). */
        fn set_huge_addr(&mut self, idx: usize, physaddr: usize, flags: PageFlags);
        /* Set the given item as absent, and clear its present flag. */
        fn set_absent(&mut self, idx: usize, data: usize);
    }
    
    // (offset is the offset for the start of the frame/subpage in physmem, measured from the base physmem address)
    pub enum PAllocItem {
        Page{index: usize, offset: usize},
        SubTable{index: usize, offset: usize, alloc: PartialPageAllocation}
    }
    impl PAllocItem {
        #[inline]
        pub fn offset(&self) -> usize {
            match self {
                PAllocItem::Page { offset, .. } | PAllocItem::SubTable { offset, .. } => *offset,
            }
        }
        #[inline]
        pub fn offset_mut(&mut self) -> &mut usize {
            match self {
                PAllocItem::Page { offset, .. } | PAllocItem::SubTable { offset, .. } => offset,
            }
        }
    }
    // PartialPageAllocation stores the indicies and offsets of page allocations internally
    pub struct PartialPageAllocation(Vec<PAllocItem>);
    impl PartialPageAllocation {
        pub fn new(items: Vec<PAllocItem>) -> Self {
            Self(items)
        }
        
        pub fn entries(&self) -> &[PAllocItem] {
            &self.0
        }
        pub fn into_entries(self) -> Vec<PAllocItem> {
            self.0
        }
        
        fn _fmt_inner(&self, dbl: &mut core::fmt::DebugList<'_,'_>, prefix: &str, parentoffset: usize){
            // Entries
            let mut previndex: [usize; 2] = [69420, 69440]; let mut prevoff: [usize; 2] = [42069, 42068];  // [recent, 2nd most recent]
            let mut doneellipsis: bool = false;
            
            #[inline(always)]
            fn fmte(dbl: &mut core::fmt::DebugList<'_,'_>, prefix: &str, idx: usize, offset: usize){
                dbl.entry(&alloc::format!("{}[{}]@+{:x}", prefix, idx, offset));
            }
            
            for item in &self.0 {
                match item {
                    entry @ &PAllocItem::Page{ index, offset } => {
                        // Page
                        // (handle consistent entries with a "...")
                        'fmtentry: {
                            if index == previndex[0]+1 {
                                // Safety: if you're seriously using the 64th bit on a physical address offset  then you're fucking mental - it's not even supported by the page table
                                let offset: isize = offset as isize;
                                let prevoff: [isize; 2] = [prevoff[0] as isize, prevoff[1] as isize];
                                
                                let prevdiff: isize = prevoff[0] - prevoff[1];
                                let curdiff: isize = offset - prevoff[0];
                                if prevdiff == curdiff {
                                    if !doneellipsis { dbl.entry(&"..."); doneellipsis = true; }
                                    // We're done here!
                                    break 'fmtentry;
                                }
                            }
                            // Else clear ... state
                            if doneellipsis {
                                doneellipsis = false;
                                fmte(dbl, prefix, previndex[1], parentoffset+prevoff[1]);
                                fmte(dbl, prefix, previndex[0], parentoffset+prevoff[0]);
                            }
                            
                            // And add current one...
                            fmte(dbl, prefix, index, parentoffset+offset);
                        }
                        previndex[1] = previndex[0]; previndex[0] = index;
                        prevoff[1] = prevoff[0]; prevoff[0] = offset;
                    }
                    suballoc @ &PAllocItem::SubTable{ index, offset, ref alloc } => {
                        // Clear ... state
                        if doneellipsis { fmte(dbl, prefix, previndex[1], parentoffset+prevoff[1]); fmte(dbl, prefix, previndex[0], parentoffset+prevoff[0]); }
                        // Sub-allocation
                        alloc._fmt_inner(dbl, &alloc::format!("{}[{}]", prefix, index), parentoffset+offset);
                    }
                }
            }
            // Clear ... state
            if doneellipsis { fmte(dbl, prefix, previndex[1], parentoffset+prevoff[1]); fmte(dbl, prefix, previndex[0], parentoffset+prevoff[0]); }
        }
    }
    impl core::fmt::Debug for PartialPageAllocation {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            let mut dbl = f.debug_list();
            self._fmt_inner(&mut dbl, "", 0);
            dbl.finish()
        }
    }
}
pub(in self) use sealed::{PageFrameAllocatorImpl,IPageTableImpl,PAllocItem,PartialPageAllocation};

#[allow(private_bounds)]
pub trait PageFrameAllocator: PageFrameAllocatorImpl {}
#[allow(private_bounds)]
impl<T: PageFrameAllocatorImpl> PageFrameAllocator for T {}

#[allow(private_bounds)]
pub trait IPageTable: IPageTableImpl {}
#[allow(private_bounds)]
impl<T: IPageTableImpl> IPageTable for T {}
