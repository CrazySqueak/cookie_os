
use x86_64::structures::paging::page_table::{PageTable,PageTableEntry,PageTableFlags};
use x86_64::addr::PhysAddr;

use super::*;
use super::impl_firstfit::MLFFAllocator;
use crate::logging::klog;

const PF_PINNED: PageTableFlags = PageTableFlags::BIT_9;

#[repr(transparent)]
pub struct X64PageTable<const LEVEL: usize>(PageTable);
impl<const LEVEL: usize> X64PageTable<LEVEL> {
    /* Returns the default flags (most restrictive). ??? TODO */
    const fn _default_flags() -> PageTableFlags {
        let mut defaults = PageTableFlags::empty();
        if cfg!(feature = "per_page_NXE_bit") { defaults = defaults.union(PageTableFlags::NO_EXECUTE); }
        defaults
    }
    fn _calc_flags<const INCLUDE_NON_TRANSITIVE: bool>(mut previous: PageTableFlags, add: &PageFlags) -> PageTableFlags {
        {
            let add = add.tflags;
            use TransitivePageFlags as TF;
            if add.contains(TF::USER_ALLOWED) { previous |=  PageTableFlags::USER_ACCESSIBLE };
            if add.contains(TF::WRITEABLE   ) { previous |=  PageTableFlags::WRITABLE        };
            if cfg!(feature="per_page_NXE_bit") && add.contains(TF::EXECUTABLE) { previous &=! PageTableFlags::NO_EXECUTE      };
        }
        if INCLUDE_NON_TRANSITIVE {
            let add = add.mflags;
            use MappingSpecificPageFlags as MF;
            if add.contains(MF::PINNED) { previous |= PF_PINNED };
            if cfg!(feature="page_global_bit") && add.contains(MF::GLOBAL) { previous |=  PageTableFlags::GLOBAL };
        }
        previous
    }
    
    fn _logging_physaddr(&self) -> usize {
        ptaddr_virt_to_phys(core::ptr::addr_of!(self.0) as usize)
    }
}
impl<const LEVEL: usize> IPageTableImpl for X64PageTable<LEVEL> {
    const NPAGES: usize = 512;
    
    fn new() -> Self {
        X64PageTable(PageTable::new())
    }
    
    fn is_unused(&self, idx: usize) -> bool {
        self.0[idx].is_unused()
    }
    
    fn get_num_pages_used(&self) -> usize {
        self.0.iter().filter(|e| !e.is_unused()).count()
    }
    
    // modification
    unsafe fn set_subtable_addr(&mut self, idx: usize, phys_addr: usize){
        let flags = Self::_default_flags() | PageTableFlags::PRESENT;
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Mapping sub-table {:x}[{}] -> {:x}", self._logging_physaddr(), idx, phys_addr);
        self.0[idx].set_addr(PhysAddr::new(phys_addr as u64), flags);
    }
    fn add_subtable_flags<const INCLUDE_NON_TRANSITIVE: bool>(&mut self, idx: usize, flags: &PageFlags){
        let flags = Self::_calc_flags::<INCLUDE_NON_TRANSITIVE>(self.0[idx].flags(), &flags);
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Setting sub-table {:x}[{}] flags to {:?}", self._logging_physaddr(), idx, flags);
        self.0[idx].set_flags(flags);
    }
    
    fn set_huge_addr(&mut self, idx: usize, physaddr: usize, flags: PageFlags){
        let flags = Self::_calc_flags::<true>(Self::_default_flags() | match LEVEL { // Set present + huge flag
            1 => PageTableFlags::PRESENT,  // (huge page flag is used for PAT on level 1 page tables)
            _ => PageTableFlags::PRESENT | PageTableFlags::HUGE_PAGE,
        }, &flags);
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Mapping entry {:x}[{}] to {:x} (flags={:?})", self._logging_physaddr(), idx, physaddr, flags);
        self.0[idx].set_addr(PhysAddr::new(physaddr as u64), flags);  // set addr
    }
    fn set_absent(&mut self, idx: usize, data: usize){
        let data = data.checked_shl(1).expect("Data value is out-of-bounds!") &!1;  // clear the "present" flag
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Mapping entry {:x}[{}] to N/A (data={:x})", ptaddr_virt_to_phys(core::ptr::addr_of!(self.0) as usize), idx, data);
        unsafe { *((&mut self.0[idx] as *mut PageTableEntry) as *mut u64) = data as u64; }  // Update entry manually
    }
}

type X64Level1 = MLFFAllocator<NoDeeper , X64PageTable<1>, false, true >;  // Page Table
type X64Level2 = MLFFAllocator<X64Level1, X64PageTable<2>, true , true >;  // Page Directory

#[cfg(not(feature="1G_huge_pages"))]
type X64Level3 = MLFFAllocator<X64Level2, X64PageTable<3>, true , false>;  // Page Directory Pointer Table
#[cfg(feature="1G_huge_pages")]
type X64Level3 = MLFFAllocator<X64Level2, X64PageTable<3>, true , true>;  // Page Directory Pointer Table

type X64Level4 = MLFFAllocator<X64Level3, X64PageTable<4>, true , false>;  // Page Map Level 4

pub(in super) type TopLevelPageAllocator = X64Level4;

/* Discard the upper 16 bits of an address (for 48-bit vmem) */
pub fn crop_addr(addr: usize) -> usize {
    addr & 0x0000_ffff_ffff_ffff
}
/* Convert a virtual address to a physical address, for use with pointing the CPU to page tables. */
pub fn ptaddr_virt_to_phys(vaddr: usize) -> usize {
    vaddr-crate::lowlevel::HIGHER_HALF_OFFSET // note: this will break if the area where the page table lives is not offset-mapped (or if the address has been cropped to hold all 0s for non-canonical bits)
}

pub(in super) unsafe fn set_active_page_table(phys_addr: usize){
    use x86_64::addr::PhysAddr;
    use x86_64::structures::paging::frame::PhysFrame;
    use x86_64::registers::control::Cr3;
    
    let (oldaddr, cr3flags) = Cr3::read();
    klog!(Debug, MEMORY_PAGING_MAPPINGS, "Switching active page table from 0x{:x} to 0x{:x}. (cr3flags={:?})", oldaddr.start_address(), phys_addr, cr3flags);
    Cr3::write(PhysFrame::from_start_address(PhysAddr::new(phys_addr.try_into().unwrap())).expect("Page Table Address Not Aligned!"), cr3flags)
}

pub(super) fn inval_tlb_pg(virt_addr: usize){
    use x86_64::instructions::tlb::flush;
    use x86_64::addr::VirtAddr;
    klog!(Debug, MEMORY_PAGING_TLB, "Flushing TLB for 0x{:x}", virt_addr);
    flush(VirtAddr::new_truncate(virt_addr.try_into().unwrap()))
}