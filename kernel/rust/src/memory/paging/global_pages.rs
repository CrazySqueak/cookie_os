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
    pub fn get_vmem_offset(&self) -> PageAlignedAddressT {
        PageAlignedAddressT::new(self.0.metadata().offset)
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
const _:() = assert!(KERNEL_PTABLE_VADDR == 0xFFFF800000000000);
pub const MMIO_PTABLE_IDX    : usize = GLOBAL_PAGES_START_IDX+1;
pub const MMIO_PTABLE_VADDR  : usize = canonical_addr(MMIO_PTABLE_IDX*TOPLEVEL_PAGE_SIZE);

pub const N_GLOBAL_TABLES: usize = 2;

pub const KERNEL_STATIC_PT_IDX: usize = 511;
pub const KERNEL_STATIC_PT_VADDR: usize = 0b0111111111000000000000000000000000000000000000000;  // offset was copy-pasted from paging-calculator so that's why it's so awfully formatted

lazy_static! {
    // Note: these are all stored in Arc<>s so they're heap allocated (phew)
    /// KERNEL_PTABLE - kernel data (heap + stack + dynamic allocations)
    pub static ref KERNEL_PTABLE: GlobalPageTable = {
        let kt = GlobalPageTable::new(KERNEL_PTABLE_VADDR, pageFlags!(t:WRITEABLE));
        _map_kernel(&kt, 0, "KERNEL_PTABLE", true);
        kt
    };
    /// MMIO_PTABLE - for MMIO
    pub static ref MMIO_PTABLE: GlobalPageTable = {
        let mt = GlobalPageTable::new(MMIO_PTABLE_VADDR, pageFlags!(t:WRITEABLE));
        mt
    };
    
    /// KERNEL_STATIC_PT - Kernel code + statics are now located at -2GiB
    pub static ref KERNEL_STATIC_PT: GlobalPageTable = {
        let kt = GlobalPageTable::new(KERNEL_STATIC_PT_VADDR, pageFlags!(t:WRITEABLE,t:EXECUTABLE));
        _map_kernel(&kt, 0b0111111110000000000000000000000000000000, "KERNEL_STATIC_PT", false);  // offset was copy-pasted from paging-calculator so that's why it's so awfully formatted
        kt
    };
    // N.B. KERNEL_STATIC_PT isn't included in N_GLOBAL_TABLES and such because it's responsible for the very end of virtual memory (rather than consecutively after the halfway mark)
    pub static ref KERNEL_STATIC_PHYSADDR: usize = ptaddr_virt_to_phys(KERNEL_STATIC_PT._begin_active().get_page_table_ptr() as usize);
    pub static ref KERNEL_STATIC_FLAGS: PageFlags = KERNEL_STATIC_PT.1;
    
    // SAFETY: all tables in this array must have the 'static lifetime
    //         also TODO: ensure they don't get moved within physmem
        static ref ALL_GLOBAL_TABLES     : [&'static GlobalPageTable; N_GLOBAL_TABLES] = [&KERNEL_PTABLE, &MMIO_PTABLE];
    pub static ref GLOBAL_TABLE_PHYSADDRS: [         usize          ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| ptaddr_virt_to_phys(pt._begin_active().get_page_table_ptr() as usize));
    pub static ref GLOBAL_TABLE_FLAGS    : [         PageFlags      ; N_GLOBAL_TABLES] = ALL_GLOBAL_TABLES.map(|pt| pt.1);
}


extern "C" { static kstack_guard_page: u8; }
/* Map the kernel to the kernel table. Should be called on initialisation. */
fn _map_kernel(kernel_ptable: &GlobalPageTable, extra_offset: usize, table_name: &str, map_data: bool){
    use crate::logging::klog;
    klog!(Info, MEMORY_PAGING_GLOBALPAGES, "Initialising {}.", table_name);
    
    let (kstart, kend) = crate::memory::physical::get_kernel_bounds();
    let ksize = PageAllocationSizeT::new_rounded(kend - kstart);
    let kvstart = PageAlignedAddressT::new(kernel_ptable.get_vmem_offset().get() + kstart + extra_offset);
    
    // Map kernel
    klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Mapping kernel into {}. (kstart={:x} kend={:x} ksize={:x} kvstart={:x})", table_name, kstart, kend, ksize, kvstart);
    let allocation = kernel_ptable.allocate_at(kvstart, ksize).expect("Error initialising kernel page: Unable to map kernel.");
    allocation.set_base_addr(kstart, pageFlags!(t:WRITEABLE,t:EXECUTABLE,m:PINNED));
    
    // Split off kernel data section
    let (kdstart, kdend) = crate::memory::physical::get_kernel_data_bounds();
    let data_size = PageAllocationSizeT::new(kdend - kdstart);
    let data_start_offset = PageAllocationSizeT::new(kdstart - kstart);
    let (before_data, allocation) = allocation.split(data_start_offset);
    debug_assert!(before_data.size() == data_start_offset);
    let (kernel_data, after_data) = allocation.split(data_size);
    debug_assert!(kernel_data.size() == data_size);
    klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Split kernel into {:x}-{:x} and {:x}-{:x} (code) and {:x}-{:x} (data).", before_data.start(), before_data.end(), after_data.start(), after_data.end(), kernel_data.start(), kernel_data.end());
    
    if map_data {
        klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Keeping data. Dropping code allocations.");
        // TODO split off guard page
        kernel_data.leak(); drop((before_data,after_data));
        // // Map guard page
        // let guard_vaddr = unsafe { core::ptr::addr_of!(kstack_guard_page) as usize };
        // let guard_offset = PageAllocationSizeT::new(guard_vaddr - allocation.start().get());
        // const GUARD_SIZE: PageAllocationSizeT = PageAllocationSizeT::new_const(4096);  // TODO: Move guard size to arch_specific section or make it its own asm-defined value
        // klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Mapping stack guard page (vaddr={:x} offset={:x})", guard_vaddr, guard_offset);
        // let (k1alloc, rem) = allocation.split(guard_offset);
        // let (guard, k2alloc) = rem.split(GUARD_SIZE);  // guard page is 4096 bytes
        // guard.set_absent(0xFA7B_EEF0);
    } else {
        klog!(Debug, MEMORY_PAGING_GLOBALPAGES, "Keeping code. Dropping data allocations.");
        before_data.leak(); after_data.leak();
        drop(kernel_data);
    }
    
    // k1alloc.leak(); guard.leak(); k2alloc.leak();
}