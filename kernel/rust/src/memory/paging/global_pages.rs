use lazy_static::lazy_static;

use super::*;
use super::arch;

const TOPLEVEL_PAGE_SIZE: usize = <arch::TopLevelPageAllocator as PageFrameAllocatorImpl>::PAGE_SIZE;
pub type GlobalPTType = <arch::TopLevelPageAllocator as PageFrameAllocatorImpl>::SubAllocType;
pub type GPageFrameAllocator = GlobalPTType;
pub struct GlobalPageTable(LockedPageAllocator<GlobalPTType>,PageFlags);
impl GlobalPageTable {
    fn new(vmemaddr: usize, flags: PageFlags) -> Self {
        let mut dopts = LPAWGOptions::new_default(); dopts.is_global_page = true;
        Self(LockedPageAllocator::new(GlobalPTType::new(), LPAMetadata { offset: canonical_addr(vmemaddr), default_options: dopts }), flags)
    }
    pub fn get_vmem_offset(&self) -> usize {
        self.0.metadata().offset
    }
    
    /* Called to leak the pointer that will be put into every page table to reference this global page mapping */
    fn _begin_active(&self) -> &GlobalPTType {
        self.0._begin_active()
    }
}
impl core::ops::Deref for GlobalPageTable {
    type Target = LockedPageAllocator<GlobalPTType>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub type GlobalPageAllocation = PageAllocation<GlobalPTType>;

pub const GLOBAL_PAGES_START_IDX: usize = GlobalPTType::NPAGES / 2;  // Index of the first globally mapped page

pub const KERNEL_PTABLE_IDX  : usize = GLOBAL_PAGES_START_IDX+0;
pub const KERNEL_PTABLE_VADDR: usize = canonical_addr(KERNEL_PTABLE_IDX*TOPLEVEL_PAGE_SIZE);
pub const MMIO_PTABLE_IDX    : usize = GLOBAL_PAGES_START_IDX+1;
pub const MMIO_PTABLE_VADDR  : usize = canonical_addr(MMIO_PTABLE_IDX*TOPLEVEL_PAGE_SIZE);

pub const N_GLOBAL_TABLES: usize = 2;
lazy_static! {
    
    pub static ref KERNEL_PTABLE: GlobalPageTable = {
        let kt = GlobalPageTable::new(KERNEL_PTABLE_VADDR, PageFlags::new(TransitivePageFlags::EXECUTABLE, MappingSpecificPageFlags::empty()));
        _map_kernel(&kt);
        kt
    };
    pub static ref MMIO_PTABLE: GlobalPageTable = {
        let mt = GlobalPageTable::new(MMIO_PTABLE_VADDR, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()));
        mt
    };
    
    // SAFETY: all tables in this array must have the 'static lifetime
    //         also TODO: ensure they don't get moved within physmem
        static ref ALL_GLOBAL_TABLES     : [&'static GlobalPageTable; N_GLOBAL_TABLES] = [&KERNEL_PTABLE, &MMIO_PTABLE];
    pub static ref GLOBAL_TABLE_PHYSADDRS: [         usize          ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| ptaddr_virt_to_phys(pt._begin_active().get_page_table_ptr() as usize));
    pub static ref GLOBAL_TABLE_FLAGS    : [         PageFlags      ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| pt.1);
}


extern "C" { static kstack_guard_page: u8; }
/* Map the kernel to the kernel table. Should be called on initialisation. */
fn _map_kernel(kernel_ptable: &GlobalPageTable){
    use crate::logging::klog;
    klog!(Info, MEMORY_PAGING_GLOBALPAGES, "Initialising KERNEL_PTABLE.");
    
    let (kstart, kend) = crate::memory::physical::get_kernel_bounds();
    let ksize = PageAlignedUsize::new_rounded(kend - kstart);
    let kvstart = kernel_ptable.get_vmem_offset() + kstart;
    
    // Map kernel
    klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Mapping kernel into KERNEL_PTABLE. (kstart={:x} kend={:x} ksize={:x} kvstart={:x})", kstart, kend, ksize, kvstart);
    let allocation = kernel_ptable.allocate_at(kvstart, ksize).expect("Error initialising kernel page: Unable to map kernel.");
    allocation.set_base_addr(kstart, PageFlags::new(TransitivePageFlags::EXECUTABLE, MappingSpecificPageFlags::PINNED));
    
    // Map guard page
    let guard_vaddr = unsafe { core::ptr::addr_of!(kstack_guard_page) as usize };
    let guard_offset = PageAlignedUsize::new(guard_vaddr - allocation.start());
    const GUARD_SIZE: PageAlignedUsize = PageAlignedUsize::new(4096);  // TODO: Move guard size to arch_specific section or make it its own asm-defined value
    klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Mapping stack guard page (vaddr={:x} offset={:x})", guard_vaddr, guard_offset);
    let (k1alloc, rem) = allocation.split(guard_offset);
    let (guard, k2alloc) = rem.split(GUARD_SIZE);  // guard page is 4096 bytes
    guard.set_absent(0xFA7B_EEF0);
    
    k1alloc.leak(); guard.leak(); k2alloc.leak();
}