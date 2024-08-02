
use alloc::vec::Vec;

use crate::logging::klog;

#[cfg_attr(target_arch = "x86_64", path = "paging_x64.rs")]
mod arch;

pub use arch::{canonical_addr,crop_addr,ptaddr_virt_to_phys};

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
        /* Returns true if the allocator is full and cannot allocate any more pages (even in sub-allocators that it controls) */
        fn is_full(&self) -> bool;
        
        /* Get a pointer to this allocator's page table.
           (used for pointing higher-level page tables to their children) */
        fn get_page_table_ptr(&self) -> *const Self::PageTableType;
        /* Get a mutable reference to this allocator's page table.
            (used for modifying the table post-allocation in a manner that is compatible with Rust's mutability rules) */
        fn get_page_table_mut(&mut self) -> &mut Self::PageTableType;
        /* Get a mutable reference to the given sub-allocator, or None if unsupported/not present. */
        fn get_suballocator_mut(&mut self, index: usize) -> Option<&mut Self::SubAllocType>;
        
        /* Attempt to allocate the requested amount of memory. */
        fn allocate(&mut self, size: usize, alloc_strat: PageAllocationStrategies) -> Option<PartialPageAllocation>;
        /* Allocate the requested amount of memory at the given virtual memory address (relative to the start of this table's jurisdiction). */
        fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PartialPageAllocation>;
        /* Deallocate the given allocation. (note: please make sure to discard the allocation afterwards) */
        fn deallocate(&mut self, allocation: &PartialPageAllocation);
        
        /* Split the given huge page into a sub-table, if possible. */
        fn split_page(&mut self, index: usize) -> Result<PartialPageAllocation,()>;
        
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
        
        /* Get the address and flags of the entry if present, or data if not present. */
        fn get_entry(&self, idx: usize) -> Result<(usize, PageFlags),usize>;
        
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
        /* Clear the given entry, setting it to zero. */
        fn set_empty(&mut self, idx: usize);
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
    // (as it is not a generic class, it also stores the size of a "page", since otherwise address calculations are impossible without locking the original allocator or making questionable guesses)
    // Entries MUST be ordered in order of offset
    pub struct PartialPageAllocation(Vec<PAllocItem>,usize);
    impl PartialPageAllocation {
        pub fn new(items: Vec<PAllocItem>, page_size: usize) -> Self {
            Self(items,page_size)
        }
        
        #[inline]
        pub fn entries(&self) -> &[PAllocItem] {
            &self.0
        }
        #[inline]
        pub fn into_entries(self) -> Vec<PAllocItem> {
            self.0
        }
        
        #[inline]
        pub fn page_size(&self) -> usize {
            self.1
        }
        /* The starting address of this allocation in VMem, relative to the corresponding page table. (0 if empty) */
        pub fn start_addr(&self) -> usize {
            if self.0.is_empty() { return 0; }
            match &self.0[0] {
                &PAllocItem::Page { index, .. } => index*self.page_size(),
                &PAllocItem::SubTable { index, ref alloc, .. } => alloc.start_addr()+(index*self.page_size()),
            }
        }
        /* The size of this allocation in VMem. */
        pub fn size(&self) -> usize {
            let mut size = 0;
            for entry in &self.0 { match entry {
                &PAllocItem::Page { .. } => size+=self.page_size(),
                &PAllocItem::SubTable { ref alloc, .. } => size+=alloc.size(),
            }}
            size
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
                        if doneellipsis { doneellipsis = false; fmte(dbl, prefix, previndex[1], parentoffset+prevoff[1]); fmte(dbl, prefix, previndex[0], parentoffset+prevoff[0]); }
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

/// Allocation strategies allow you to add limits or adjustments to how random allocations are carried out
/// Note: These are only suggestions to the allocator, and are not strictly enforced if performance/design do not allow it. 
#[derive(Debug,Clone)]
pub struct PageAllocationStrategy {
    /// Search for a place to allocate in the reverse direction
    pub reverse_order: bool,
    /// Prevent allocation sooner than a given page (as an index, inclusive)
    pub min_page: Option<usize>,
    /// Prevent allocation later than a given page (as an index, inclusive)
    pub max_page: Option<usize>,
    /// If enabled, try to allocate in completely free sub-tables, rather than picking the first one that has enough space. (if this fails, it will switch to allowing any)
    /// This is only really relevant for allocations smaller than a single page at the given level
    pub spread_mode: bool,
}
impl PageAllocationStrategy {
    #[inline(always)]
    pub const fn new_default() -> Self {
        Self { reverse_order: false, min_page: None, max_page: None, spread_mode: false }
    }
    
    #[inline(always)]
    pub const fn reverse_order(mut self, r: bool) -> Self { self.reverse_order = r; self }
    #[inline(always)]
    pub const fn min_page(mut self, r: usize) -> Self { self.min_page = Some(r); self }
    #[inline(always)]
    pub const fn no_min_page(mut self) -> Self { self.min_page = None; self }
    #[inline(always)]
    pub const fn max_page(mut self, r: usize) -> Self { self.max_page = Some(r); self }
    #[inline(always)]
    pub const fn no_max_page(mut self) -> Self { self.max_page = None; self }
    #[inline(always)]
    pub const fn spread_mode(mut self, r: bool) -> Self { self.spread_mode = r; self }
}
// Each level you descend in the table uses the next one along in the slice. The final one is used repeatedly if needed.
pub type PageAllocationStrategies<'a> = &'a [PageAllocationStrategy];
#[inline(always)]
pub(self) fn pas_next_level_down<'a>(strat: PageAllocationStrategies<'a>) -> PageAllocationStrategies<'a> {
    if strat.len() > 1 { &strat[1..] }
    else { strat }
}
#[inline(always)]
pub(self) fn pas_current<'a>(strat: PageAllocationStrategies<'a>) -> &'a PageAllocationStrategy {
    &strat[0]
}

// There is a kernel stack strategy but no kernel heap strategy, because quite a few items on the kernel heap (e.g. page tables) expect to be offset-mapped.
// As a result, the heap is usually allocated in physical memory first, and then allocated in vmem using allocate_at. (allocate_at does not use allocation strategies as the location has already been chosen)
// (PagingContexts are not an issue even on the stack, as the Arc<> internally always allocates on the heap (as it's the only way multiple ownership can function)
pub use arch::{KALLOCATION_KERNEL_STACK,ALLOCATION_USER_STACK,ALLOCATION_USER_HEAP};
// The default strategy contains no restrictions or special behaviour
// It is useful for e.g. calling allocate(ST::PAGE_SIZE) or if no strategy should be applied
pub const ALLOCATION_DEFAULT: PageAllocationStrategies = &[PageAllocationStrategy::new_default()];