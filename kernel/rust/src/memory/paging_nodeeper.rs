
use super::*;

macro_rules! nope {
    () => { panic!("Subtables are not supported for this type! (this method should not have been called)"); }
}

/* compiler doesn't allow me to specify OR so uh, here you go. a struct that says "no more subpages" and panics if it is called
Useful for types such as MLFFAllocator which always require a subtable type due to the above restrictions. */
pub struct NoDeeper{}
impl PageFrameAllocator for NoDeeper {
    const NPAGES: usize = 0;
    const PAGE_SIZE: usize = 0;
    type PageTableType = NoDeeper;
    type SubAllocType = NoDeeper;
    fn new() -> Self { nope!(); }
    fn get_num_pages_used(&self) -> usize { nope!(); }
    fn get_page_table_ptr(&self) -> *const Self::PageTableType { nope!(); }
    fn get_page_table_mut(&mut self) -> &mut Self::PageTableType { nope!(); }
    fn get_suballocator_mut(&mut self, index: usize) -> Option<&mut Self::SubAllocType> { nope!(); }
    fn allocate(&mut self, size: usize) -> Option<PageAllocation> { nope!(); }
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PageAllocation> { nope!(); }
}
impl IPageTable for NoDeeper {
    const NPAGES: usize = 0;
    
    fn new() -> Self { nope!(); }
    fn is_unused(&self, idx: usize) -> bool { nope!(); }
    fn get_num_pages_used(&self) -> usize { nope!(); }
    unsafe fn alloc_huge(&mut self, idx: usize) { nope!(); }
    unsafe fn alloc_subtable(&mut self, idx: usize, phys_addr: usize) { nope!(); }
    unsafe fn set_addr(&mut self, idx: usize, physaddr: usize) { nope!(); }
    unsafe fn activate(&self) { nope!(); }
}