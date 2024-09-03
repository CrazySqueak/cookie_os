
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
            if add.contains(TF::USER_READABLE ) { previous |=  PageTableFlags::USER_ACCESSIBLE };
            if add.contains(TF::USER_WRITEABLE) { previous |=  PageTableFlags::WRITABLE        };
            if cfg!(feature="per_page_NXE_bit") && add.contains(TF::EXECUTABLE) { previous &=! PageTableFlags::NO_EXECUTE      };
        }
        if INCLUDE_NON_TRANSITIVE {
            let add = add.mflags;
            use MappingSpecificPageFlags as MF;
            if add.contains(MF::PINNED             ) { previous |= PF_PINNED                     };
            if add.contains(MF::CACHE_DISABLE      ) { previous |= PageTableFlags::NO_CACHE      };
            if add.contains(MF::CACHE_WRITE_THROUGH) { previous |= PageTableFlags::WRITE_THROUGH };
            if cfg!(feature="page_global_bit") && add.contains(MF::GLOBAL) { previous |=  PageTableFlags::GLOBAL };
        }
        previous
    }
    fn _deser_flags(flags: PageTableFlags) -> PageFlags {
        PageFlags::new(
            {
                use TransitivePageFlags as TF; let mut tf = TF::empty();
                if  flags.contains(PageTableFlags::USER_ACCESSIBLE) { tf |= TF::USER_READABLE }
                if  flags.contains(PageTableFlags::WRITABLE       ) { tf |= TF::USER_WRITEABLE}
                if !flags.contains(PageTableFlags::NO_EXECUTE     ) { tf |= TF::EXECUTABLE    }
                tf
            },
            {
                use MappingSpecificPageFlags as MF; let mut mf = MF::empty();
                if  flags.contains(PageTableFlags::GLOBAL)        { mf |= MF::GLOBAL             }
                if  flags.contains(PF_PINNED)                     { mf |= MF::PINNED             }
                if  flags.contains(PageTableFlags::NO_CACHE)      { mf |= MF::CACHE_DISABLE      }
                if  flags.contains(PageTableFlags::WRITE_THROUGH) { mf |= MF::CACHE_WRITE_THROUGH}
                mf
            },
        )
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
    
    fn get_entry(&self, idx: usize) -> Result<(usize, PageFlags),usize> {
        let flags = self.0[idx].flags();
        if flags.contains(PageTableFlags::PRESENT) {
            let addr: usize = self.0[idx].addr().as_u64().try_into().unwrap();
            let flags = Self::_deser_flags(flags);
            Ok((addr, flags))
        } else {
            let data = unsafe { *((&self.0[idx] as *const PageTableEntry) as *const u64) } >> 1;
            Err(data.try_into().unwrap())
        }
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
    fn set_empty(&mut self, idx: usize){
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Clearing entry {:x}[{}]", ptaddr_virt_to_phys(core::ptr::addr_of!(self.0) as usize), idx);
        self.0[idx].set_unused()
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
/// The range of memory covered by an entry in the lowest-level page table
pub const MIN_PAGE_SIZE: usize = X64Level1::PAGE_SIZE;

// Kernel Stack: In the kernel page
pub const KALLOCATION_KERNEL_STACK: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true), PageAllocationStrategy::new_default().reverse_order(true).spread_mode(true), PageAllocationStrategy::new_default().reverse_order(true)];
// Kernel Dynamic Allocations in the MMIO Page: In the mmio page, in reverse order to avoid clashing with offset mapped stuff
pub const KALLOCATION_DYN_MMIO: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true), PageAllocationStrategy::new_default()];

// User Stack: R2L before the kernel pages, spread mode
pub const ALLOCATION_USER_STACK: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true).max_page(255), PageAllocationStrategy::new_default().reverse_order(true).spread_mode(true), PageAllocationStrategy::new_default().reverse_order(true)];
// User Heap: Start 1G inwards
pub const ALLOCATION_USER_HEAP: PageAllocationStrategies = &[PageAllocationStrategy::new_default(), PageAllocationStrategy::new_default().min_page(1), PageAllocationStrategy::new_default()];

// methods
/* Discard the upper 16 bits of an address (for 48-bit vmem) */
pub fn crop_addr(addr: usize) -> usize {
    addr & 0x0000_ffff_ffff_ffff
}
/* Convert a virtual address to a physical address, for use with pointing the CPU to page tables. */
pub fn ptaddr_virt_to_phys(vaddr: usize) -> usize {
    vaddr-super::global_pages::KERNEL_PTABLE_VADDR // note: this will break if the area where the page table lives is not offset-mapped (or if the address has been cropped to hold all 0s for non-canonical bits)
}

/* Ensure a virtual address is canonical */
#[inline(always)]
pub const fn canonical_addr(vaddr: usize) -> usize {
    x86_64::VirtAddr::new_truncate(vaddr as u64).as_u64() as usize
}

pub(in super) unsafe fn set_active_page_table(phys_addr: usize){
    use x86_64::addr::PhysAddr;
    use x86_64::structures::paging::frame::PhysFrame;
    use x86_64::registers::control::Cr3;
    
    let (oldaddr, cr3flags) = Cr3::read();
    klog!(Debug, MEMORY_PAGING_MAPPINGS, "Switching active page table from 0x{:x} to 0x{:x}. (cr3flags={:?})", oldaddr.start_address(), phys_addr, cr3flags);
    Cr3::write(PhysFrame::from_start_address(PhysAddr::new(phys_addr.try_into().unwrap())).expect("Page Table Address Not Aligned!"), cr3flags)
}

// allocation, voffset - define the vmem addresses to invalidate TLB mappings for
// include_global - If true, include global pages as well
// cpu_nums - If Some, assumed to be a broadcast, with the CPUs to invalidate for (if targets may be selected). If None, assumed to be local only (no broadcast is made)
pub(super) fn inval_tlb_pg(allocation: &super::PartialPageAllocation, voffset: usize, include_global: bool, cpu_nums: Option<&[usize]>){
    use x86_64::structures::paging::page::{Size4KiB,Size2MiB};
    let vmem_start = allocation.start_addr()+voffset; let vmem_end_xcl = allocation.end_addr()+voffset; let length = vmem_end_xcl-vmem_start;
    klog!(Debug, MEMORY_PAGING_TLB, "Flushing TLB for 0x{:x}..0x{:x}", vmem_start, vmem_end_xcl);
    
    if cfg!(feature = "enable_amd64_invlpgb") && INVLPGB.is_some() && cpu_nums.is_some() {
        // Use INVLPGB instruction if enabled
        if vmem_start%X64Level2::PAGE_SIZE == 0 && length%X64Level2::PAGE_SIZE == 0 {
            klog!(Debug, MEMORY_PAGING_TLB, "Flushing using call_invlpgb (size=2MiB).");
            call_invlpgb::<Size2MiB>(vmem_start, vmem_end_xcl, include_global)
        } else {
            klog!(Debug, MEMORY_PAGING_TLB, "Flushing using call_invlpgb (size=4KiB).");
            call_invlpgb::<Size4KiB>(vmem_start, vmem_end_xcl, include_global)
        }
    } else if crate::coredrivers::system_apic::is_local_apic_initialised() && cpu_nums.is_some() {
        // Broadcast invalidation over APIC (using interrupts)
        klog!(Debug, MEMORY_PAGING_TLB, "Flushing using APIC.");
        let local_num = crate::multitasking::get_cpu_num();
        let cpu_nums = cpu_nums.unwrap();
        if include_global {
            // This affects all page mappings. No comparisons on CPU IDs need to be made
            // Invalidate locally
            klog!(Debug, MEMORY_PAGING_TLB_APIC, "Flushing global page locally using call_invlpg_recursive.");
            call_invlpg_recursive(allocation, allocation.start_addr()+voffset);
            // Invalidate globally
            klog!(Debug, MEMORY_PAGING_TLB_APIC, "Flushing global page for all other CPUs using APIC interrupt.");
            let al = alloc::sync::Arc::new(allocation.clone());
            for cpu_num in cpu_nums.iter() {
                if *cpu_num == local_num { continue; }
                super::push_shootdown(*cpu_num,alloc::sync::Arc::clone(&al),voffset,include_global);
            }
            crate::lowlevel::broadcast_shootdown();
        } else {
            // Invalidate locally
            if cpu_nums.contains(&local_num) {
                klog!(Debug, MEMORY_PAGING_TLB_APIC, "Flushing locally for CPU{} using call_invlpg_recursive.", local_num);
                call_invlpg_recursive(allocation, allocation.start_addr()+voffset);
            }
            // Broadcast to target CPUs over APIC
            let al = alloc::sync::Arc::new(allocation.clone());
            for cpu_num in cpu_nums.iter() {
                if *cpu_num == local_num { continue; }
                klog!(Debug, MEMORY_PAGING_TLB_APIC, "Flushing for CPU{} using APIC interrupt.", local_num);
                super::push_shootdown(*cpu_num,alloc::sync::Arc::clone(&al),voffset,include_global);
                crate::lowlevel::send_shootdown_cpunum(*cpu_num);
            }
        }
    } else {
        // Invalidate using the old-fashioned way
        klog!(Debug, MEMORY_PAGING_TLB, "Flushing locally using call_invlpg_recursive.");
        call_invlpg_recursive(allocation, allocation.start_addr()+voffset);
        //panic!("No supported TLB invalidation strategy enabled?!")
    }
}

lazy_static::lazy_static! {
    static ref INVLPGB: Option<x86_64::instructions::tlb::Invlpgb> = x86_64::instructions::tlb::Invlpgb::new();
}
fn call_invlpgb<S: x86_64::structures::paging::page::NotGiantPageSize>(vmem_start: usize, vmem_end_xcl: usize, include_global: bool){
    use x86_64::addr::VirtAddr;
    use x86_64::structures::paging::page::{PageRange,Page};
    use x86_64::instructions::tlb::InvlpgbFlushBuilder;
    let range = PageRange {
        start: Page::<S>::from_start_address(VirtAddr::new(vmem_start.try_into().unwrap())).expect("Provided TLB flush start address not page-aligned!"),
        end: Page::<S>::from_start_address(VirtAddr::new(vmem_end_xcl.try_into().unwrap())).expect("Provided TLB flush end address not page-aligned!"),
    };
    
    let mut flush = INVLPGB.as_ref().unwrap().build().pages(range);
    if include_global {flush.include_global();}
    flush.flush();
}

fn call_invlpg_recursive(allocation: &super::PartialPageAllocation, voffset: usize){
    use x86_64::instructions::tlb::flush;
    use x86_64::VirtAddr;
    for item in allocation.entries() {
        match item {
            &PAllocItem::Page { index, offset } => {
                klog!(Debug, MEMORY_PAGING_TLB_RECUR, "Flushing addr 0x{:x} (vo={:x} o={:x})", voffset+offset, voffset, offset);
                flush(VirtAddr::new((voffset + offset).try_into().unwrap()));
            },
            &PAllocItem::SubTable { offset, alloc: ref suballocation, .. } => {
                klog!(Debug, MEMORY_PAGING_TLB_RECUR, "Recursing with offset 0x{:x} (vo={:x} o={:x})", voffset+offset, voffset, offset);
                call_invlpg_recursive(suballocation, voffset + offset);
            },
        }
    }
}
