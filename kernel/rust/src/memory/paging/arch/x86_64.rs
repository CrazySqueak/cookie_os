use alloc::string::{String, ToString};
use ::x86_64 as x86_64;
use lazy_static::lazy_static;
use x86_64::structures::paging::page_table::{PageTable, PageTableEntry, PageTableFlags};
use x86_64::addr::PhysAddr;
use x86_64::instructions::tlb::{InvPicdCommand, Invlpgb, Pcid, PcidTooBig};
use x86_64::{instructions, VirtAddr};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{page, PageSize};
use x86_64::structures::paging::page::PageRange;
use crate::memory::paging as paging_root;
use paging_root::*;
use paging_root::impl_firstfit::MLFFAllocator;
use crate::logging::klog;
use crate::memory::paging::tlb::{ActivePageID, AddressSpaceID};
use crate::multitasking::cpulocal::CpuLocal;
use crate::multitasking::disable_interruptions;

impl Into<VirtAddr> for PageAlignedAddressT {
    fn into(self) -> VirtAddr {
        VirtAddr::new(self.get() as u64)
    }
}
pub const MAX_ASID: u16 = 4095;
impl Into<Option<Pcid>> for AddressSpaceID {
    fn into(self) -> Option<Pcid> {
        match self {
            AddressSpaceID::Unassigned => None,
            #[cfg(feature="__IallowNonZeroASID")]
            AddressSpaceID::Assigned(x) => {
                Some(Pcid::new(x.get()).unwrap())
            },
        }
    }
}
impl Into<Pcid> for AddressSpaceID {
    fn into(self) -> Pcid {
        Pcid::new(self.into_u16()).unwrap()
    }
}

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
            if add.contains(TF::WRITEABLE     ) { previous |=  PageTableFlags::WRITABLE        };
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
                if  flags.contains(PageTableFlags::WRITABLE       ) { tf |= TF::WRITEABLE     }
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
        let data = data.checked_shl(1).expect("Data value is out-of-bounds!") &!1;  // clear the "present" flag (TODO: reserve bit 2 for "is swapped out" / "is guard")
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

pub type TopLevelPageAllocator = X64Level4;
/// The range of memory covered by an entry in the lowest-level page table
pub const MIN_PAGE_SIZE: usize = X64Level1::PAGE_SIZE;

// Kernel Stack: In the kernel page
pub const KALLOCATION_KERNEL_STACK: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true), PageAllocationStrategy::new_default().reverse_order(true).spread_mode(true), PageAllocationStrategy::new_default().reverse_order(true)];
// Kernel Dynamic Allocations (general usage)
pub const KALLOCATION_KERNEL_GENERALDYN: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true)];
// Kernel Dynamic Allocations in the MMIO Page: In the mmio page, in reverse order to avoid clashing with offset mapped stuff
pub const KALLOCATION_DYN_MMIO: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true), PageAllocationStrategy::new_default()];

// User Stack: R2L before the kernel pages, spread mode
pub const ALLOCATION_USER_STACK: PageAllocationStrategies = &[PageAllocationStrategy::new_default().reverse_order(true).max_page(255), PageAllocationStrategy::new_default().reverse_order(true).spread_mode(true), PageAllocationStrategy::new_default().reverse_order(true)];
// User Heap: Start 1G inwards
pub const ALLOCATION_USER_HEAP: PageAllocationStrategies = &[PageAllocationStrategy::new_default(), PageAllocationStrategy::new_default().min_page(1), PageAllocationStrategy::new_default()];

// utility methods
/* Discard the upper 16 bits of an address (for 48-bit vmem) */
pub fn crop_addr(addr: usize) -> usize {
    addr & 0x0000_ffff_ffff_ffff
}
/* Convert a virtual address to a physical address, for use with pointing the CPU to page tables. */
pub fn ptaddr_virt_to_phys(vaddr: usize) -> usize {
    debug_assert!(vaddr >= paging_root::global_pages::KERNEL_PTABLE_VADDR, "Page Table contents stored below KERNEL_PTABLE in vmem");
    debug_assert!(vaddr < paging_root::global_pages::KERNEL_PTABLE_VADDR + TopLevelPageAllocator::PAGE_SIZE, "Page Table contents stored after KERNEL_PTABLE in vmem");
    vaddr-paging_root::global_pages::KERNEL_PTABLE_VADDR // note: this will break if the area where the page table lives is not offset-mapped (or if the address has been cropped to hold all 0s for non-canonical bits)
}

/* Ensure a virtual address is canonical */
#[inline(always)]
pub const fn canonical_addr(vaddr: usize) -> usize {
    x86_64::VirtAddr::new_truncate(vaddr as u64).as_u64() as usize
}

pub(in crate::memory::paging) fn is_asid_supported() -> bool {
    cfg!(feature="enable_x86_64_pcid") && false  // TODO
}
// Instructions
/// Set the current active page table by writing to Cr3.
///
/// * `phys_addr` - physical address of the PML4 table
/// * `asid` - Address Space ID (PCID) to use
/// * `flush` - Whether the TLB is required to be flushed (e.g. because a PCID is being re-used).
///
/// If `flush` is `false`, and (oldaddr,oldasid) == (phys_addr,asid), then no flush or write to Cr3 will occur.
/// If `asid` is [Assigned](AddressSpaceID::Assigned), then the cached entries for that PCID will be re-used (if flush is false) or cleared (if flush is true).
/// If `asid` is [Unassigned](AddressSpaceID::Unassigned), then the cached entries for that PCID will always be flushed.
pub(in crate::memory::paging) unsafe fn set_active_page_table(phys_addr: usize, asid: AddressSpaceID, flush: bool){
    use x86_64::addr::PhysAddr;
    use x86_64::structures::paging::frame::PhysFrame;
    use x86_64::registers::control::Cr3;
    
    let (oldaddr, old_pcid) = Cr3::read_raw();
    let newaddr = PhysFrame::from_start_address(PhysAddr::new(phys_addr as u64)).expect("Page Table Address Not Aligned!");

    // Don't bother if this would have no effect and flush is false
    if (!flush) && (oldaddr,old_pcid) == (newaddr,asid.into_u16()) {
        klog!(Debug, MEMORY_PAGING_MAPPINGS, "Not switching page table, as both are identical and flush is false.");
        return;
    }

    klog!(Info, MEMORY_PAGING_MAPPINGS, "Switching active page table from 0x{:06x}[{:03x}] to 0x{:06x}[{}].",
            oldaddr.start_address(), old_pcid,
            phys_addr, if is_asid_supported() { alloc::format!("{:03x}",asid.into_u16()) } else { String::from("N/A") });
    if is_asid_supported() {
        let pcid: Option<Pcid> = asid.into();
        #[cfg_attr(not(feature="__IallowNonZeroASID"), expect(irrefutable_let_patterns))]
        if let AddressSpaceID::Unassigned = asid {
            // write_raw clears the top bit, ensuring that a flush occurs
            klog!(Debug, MEMORY_PAGING_MAPPINGS, "Flushing on switch to Unassigned.");
            Cr3::write_raw(newaddr, 0);
        } else if flush {
            klog!(Debug, MEMORY_PAGING_MAPPINGS, "Flushing on switch to {:?}.", pcid);
            Cr3::write_pcid(newaddr, pcid.unwrap())
        } else {
            klog!(Debug, MEMORY_PAGING_MAPPINGS, "Not flushing on switch to {:?}.", pcid);
            Cr3::write_pcid_no_flush(newaddr, pcid.unwrap())
        }
    } else {
        debug_assert!(matches!(asid, AddressSpaceID::Unassigned));
        Cr3::write_raw(newaddr, 0)
    }
}

/// Set the current active page ID
pub(in crate::memory::paging) fn set_active_id(active_page_id: ActivePageID){
    // TODO
}

/// Invalidate a set of pages in the local CPU's TLB.
///
/// * `allocation` - The allocation to invalidate the mappings for.
/// * `asid = Some(x)` - The address space to invalidate the mappings for (may skip those tagged global).
/// * `asid = None` - Invalidate the given global mappings.
pub(in crate::memory::paging) fn inval_local_tlb_pg(allocation: PPAandOffset, asid: Option<AddressSpaceID>) {
    // asid.is_some() = If we are restricting based on ASID
    // If true, then this is for a specific address space.
    // If false, then this is for global mappings.

    #[derive(Debug,Clone,Copy)]
    struct InvalStrategy {
        /// True if CR3.PCID != asid, (and thus a simple INVLPG won't work)
        different_asid: bool,
        /// True if INVPCID is supported on this CPU
        invpcid_supported: bool,
    }
    let mut invstrat = InvalStrategy {
        // If asid is some() and different to our current PCID, then we must invalidate for a different PCID
        different_asid: is_asid_supported() && asid.is_some() && (Cr3::read_raw().1 != asid.unwrap().into_u16()),
        // If invpcid is supported
        invpcid_supported: is_asid_supported() && false,
    };

    // Disable interruptions while we handle this
    let no_interruptions = disable_interruptions();

    // Switch pcid if we need to change it + changing for another PCID is unsupported
    let old_pcid = if asid.is_some() && invstrat.different_asid && !invstrat.invpcid_supported {
        klog!(Debug, MEMORY_PAGING_TLB, "Temporarily switching PCIDs...");
        let (oldaddr, oldpcid) = Cr3::read_pcid();
        // SAFETY: This is safe because we're only using kernel memory (mapped globally) while this is active
        //          + we're setting it to the same page table as it was already set to
        unsafe { Cr3::write_pcid_no_flush(oldaddr,asid.unwrap().into()) }
        // Update the strategy - as we can now use INVLPG like normal
        invstrat.different_asid = false;
        Some(oldpcid)
    } else { None };

    // Perform invalidation
    __inner(allocation.ppa, asid, allocation.offset, invstrat);
    fn __inner(allocation: &PartialPageAllocation, asid: Option<AddressSpaceID>, voffset: usize, invstrat: InvalStrategy) {
        for item in allocation.entries() {
            match item {
                &PAllocItem::Page { index, offset } => {
                    klog!(Debug, MEMORY_PAGING_TLB_RECUR, "Flushing addr 0x{:x} (vo={:x} o={:x})", voffset+offset, voffset, offset);

                    let virt_addr = VirtAddr::new((voffset + offset).try_into().unwrap());
                    if asid.is_some() && invstrat.different_asid {
                        let pcid: Pcid = asid.unwrap().into();
                        if invstrat.invpcid_supported {
                            // SAFETY: One must ensure that invpcid_supported is only true if it is supported by the host CPU
                            unsafe { instructions::tlb::flush_pcid(InvPicdCommand::Address(virt_addr, pcid)) }
                        } else {
                            unreachable!("This should never be reached! Restricted by PCID + different PCID, but INVPCID is not supported and we haven't switched?")
                        }
                    } else {
                        // This invalidates for the current address space + the global address space,
                        // thus meaning that it works for both asid=Some(current) and asid=None
                        instructions::tlb::flush(virt_addr);
                    }
                },
                &PAllocItem::SubTable { offset, alloc: ref suballocation, .. } => {
                    klog!(Debug, MEMORY_PAGING_TLB_RECUR, "Recursing with offset 0x{:x} (vo={:x} o={:x})", voffset+offset, voffset, offset);
                    __inner(suballocation, asid, voffset + offset, invstrat);
                },
            }
        }
    }

    if let Some(oldpcid) = old_pcid {  // switch back now
        klog!(Debug, MEMORY_PAGING_TLB, "Switching PCID back to original...");
        let (oldaddr, _) = Cr3::read_raw();
        // SAFETY: This is safe because we're returning to our original state
        unsafe { Cr3::write_pcid_no_flush(oldaddr,oldpcid) }
        invstrat.different_asid = true;
    }
}

lazy_static! {
    static ref INVLPGB: Option<Invlpgb> = Invlpgb::new();
}
/// Remotely invalidate a set of pages on all applicable CPUs' TLBs (for a given ASID).\
/// Returns true if this was successful.\
/// Returns false if this was unsuccessful (e.g. unsupported),
/// and must be done using [push_flushes](crate::memory::paging::tlb::push_flushes) + an IPI broadcast instead.
///
/// * `allocation` - The allocation to flush the TLB for.
/// * `asids` - The CPU-ASID mappings, containing the ASIDs to invalidate for on each CPU (CPUs which are not present are skipped).
pub(in crate::memory::paging) fn inval_tlb_pg_broadcast(active_id: Option<ActivePageID>, allocation: PPAandOffset, asids: &ClASIDs) -> bool {
    // You really think I can be asked to implement INVLPGB for non-globals atm??
    false
}
/// Remotely invalidate a set of pages on all applicable CPUs' TLBs (for the global address space).\
/// Returns true if this was successful.\
/// Returns false if this was unsuccessful (e.g. unsupported),
/// and must be done using [push_global_flushes](crate::memory::paging::tlb::push_global_flushes) + an IPI broadcast instead.
pub(in crate::memory::paging) fn inval_tlb_pg_broadcast_global(allocation: PPAandOffset) -> bool {
    if let Some(invlpgb) = INVLPGB.as_ref() {
        let start = (allocation.ppa.start_addr() + allocation.offset) as u64;
        let end = (allocation.ppa.end_addr() + allocation.offset) as u64;

        fn __flush<S:page::NotGiantPageSize>(invlpgb: &Invlpgb, start:u64, end:u64) {
            let range: PageRange<S> = PageRange {
                start: page::Page::from_start_address(VirtAddr::new(start)).unwrap(),
                end: page::Page::from_start_address(VirtAddr::new(start)).unwrap(),
            };

            invlpgb.build()
                .pages(range)
                .include_global()
                .flush();
        }

        if start%page::Size2MiB::SIZE == 0 && end%page::Size2MiB::SIZE == 0 {
            __flush::<page::Size2MiB>(invlpgb, start, end);
        } else {
            __flush::<page::Size4KiB>(invlpgb, start, end);
        }
        true
    } else {
        false
    }
}