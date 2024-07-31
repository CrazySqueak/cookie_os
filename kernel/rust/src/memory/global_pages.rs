use lazy_static::lazy_static;
use spin::RwLockReadGuard;

use super::*;
use super::arch;

const TOPLEVEL_PAGE_SIZE: usize = <arch::TopLevelPageAllocator as PageFrameAllocatorImpl>::PAGE_SIZE;
type GlobalPTType = <arch::TopLevelPageAllocator as PageFrameAllocatorImpl>::SubAllocType;
pub struct GlobalPageTable(LockedPageAllocator<GlobalPTType>,PageFlags);
impl GlobalPageTable {
    fn new(vmemaddr: usize, flags: PageFlags) -> Self {
        Self(LockedPageAllocator::new(GlobalPTType::new(), LPAMetadata { offset: vmemaddr }), flags)
    }
    pub fn get_vmem_offset(&self) -> usize {
        self.0.metadata().offset
    }
    
    /* Called to leak the pointer that will be put into every page table to reference this global page mapping */
    fn _begin_active(&self) -> &GlobalPTType {
        self.0._begin_active()
    }
    
    pub fn read(&self) -> RwLockReadGuard<GlobalPTType> {
        self.0.read()
    }
    /* This method is for testing. It will ALWAYS deadlock. */
    pub fn write(&self) -> LPageAllocatorRWLWriteGuard<GlobalPTType> {
        let mut guard = self.0.write();
        guard.options.is_global_page = true;
        guard
    }
    /* Write when active!!! */
    pub fn write_when_active(&self) -> LPageAllocatorUnsafeWriteGuard<GlobalPTType> {
        let mut guard = self.0.write_when_active();
        guard.options.is_global_page = true;
        guard
    }
}
// TODO

pub const GLOBAL_PAGES_START_IDX: usize = GlobalPTType::NPAGES / 2;  // Index of the first globally mapped page

pub const KERNEL_PTABLE_IDX  : usize = GLOBAL_PAGES_START_IDX+0;
pub const KERNEL_PTABLE_VADDR: usize = KERNEL_PTABLE_IDX*TOPLEVEL_PAGE_SIZE;

pub const N_GLOBAL_TABLES: usize = 1;
lazy_static! {
    
    pub static ref KERNEL_PTABLE: GlobalPageTable = GlobalPageTable::new(KERNEL_PTABLE_VADDR, PageFlags::new(TransitivePageFlags::EXECUTABLE, MappingSpecificPageFlags::empty()));
    
    // SAFETY: all tables in this array must have the 'static lifetime
    //         also TODO: ensure they don't get moved within physmem
        static ref ALL_GLOBAL_TABLES     : [&'static GlobalPageTable; N_GLOBAL_TABLES] = [&KERNEL_PTABLE];
    pub static ref GLOBAL_TABLE_PHYSADDRS: [         usize          ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| ptaddr_virt_to_phys(pt._begin_active().get_page_table_ptr() as usize));
    pub static ref GLOBAL_TABLE_FLAGS    : [         PageFlags      ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| pt.1);
}