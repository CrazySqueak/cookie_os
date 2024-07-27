use lazy_static::lazy_static;
use spin::RwLockReadGuard;

use super::*;
use super::arch;

type GlobalPTType = <arch::TopLevelPageAllocator as PageFrameAllocatorImpl>::SubAllocType;
pub struct GlobalPageTable(LockedPageAllocator<GlobalPTType>);
impl GlobalPageTable {
    pub fn new(vmemaddr: usize) -> Self {
        Self(LockedPageAllocator::new(GlobalPTType::new(), LPAMetadata { offset: vmemaddr }))
    }
    
    fn read(&self) -> RwLockReadGuard<GlobalPTType> {
        self.0.read()
    }
}
// TODO

pub const GLOBAL_PAGES_START_IDX: usize = GlobalPTType::NPAGES / 2;  // Index of the first globally mapped page

pub const KERNEL_PTABLE_IDX  : usize = GLOBAL_PAGES_START_IDX+0;
pub const KERNEL_PTABLE_VADDR: usize = KERNEL_PTABLE_IDX*GlobalPTType::PAGE_SIZE;

lazy_static! {
    
    pub static ref KERNEL_PTABLE: GlobalPageTable = GlobalPageTable::new(KERNEL_PTABLE_VADDR);
    
    // SAFETY: all tables in this array must have the 'static lifetime
    //         also TODO: ensure they don't get moved within physmem
    pub static ref GLOBAL_TABLE_PHYSADDRS: [usize; 1] = [&KERNEL_PTABLE].map(|pt| ptaddr_virt_to_phys(RwLockReadGuard::leak(pt.read()).get_page_table_ptr() as usize));
}