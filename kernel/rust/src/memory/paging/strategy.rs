
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
pub use super::arch::{KALLOCATION_KERNEL_STACK,KALLOCATION_DYN_MMIO,ALLOCATION_USER_STACK,ALLOCATION_USER_HEAP};
// The default strategy contains no restrictions or special behaviour
// It is useful for e.g. calling allocate(ST::PAGE_SIZE) or if no strategy should be applied
pub const ALLOCATION_DEFAULT: PageAllocationStrategies = &[PageAllocationStrategy::new_default()];