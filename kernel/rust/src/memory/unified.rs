
use core::ops::{Deref,DerefMut};
use alloc::boxed::Box;
use alloc::vec::Vec; use alloc::vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use core::fmt::{Debug, Formatter};
use super::paging::{LockedPageAllocator, PageFrameAllocator, AnyPageAllocation, PageAllocation, PageAlignedValue, PageAllocationSizeT, PageAlignedOffsetT, PageAlignedAddressT, PageFlags, pageFlags};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::{YMutex, YMutexGuard, ArcYMutexGuard, MappedYMutexGuard};
use crate::logging::klog;

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

macro_rules! add_offset_and_size {
    (($offset:expr) + ($size:expr)) => {
        {
            let l0: PageAlignedOffsetT = $offset;
            let r0: PageAllocationSizeT = $size;
            let r1: usize = r0.into();
            let r2: isize = r1.try_into().unwrap();
            let r3: PageAlignedOffsetT = r2.try_into().unwrap();
            l0 + r3
        }
    };
    (($offset:expr) - ($size:expr)) => {
        {
            let l0: PageAlignedOffsetT = $offset;
            let r0: PageAllocationSizeT = $size;
            let r1: usize = r0.into();
            let r2: isize = r1.try_into().unwrap();
            let r3: PageAlignedOffsetT = r2.try_into().unwrap();
            l0 - r3
        }
    };
}

#[derive(Clone,Copy,Debug)]
pub enum GuardPageType {
    StackLimit = 0xF47B33F,  // Fat Beef
    NullPointer = 0x4E55_4C505452,  // "NULPTR"
}
/// The type of allocation requested
#[derive(Clone,Copy,Debug)]
pub enum AllocationType {
    /// RAM - starts uninitialized
    UninitMem,
    /// RAM - starts zeroed
    ZeroedMem,
    
    /// Guard Page - attempting to access it is an error (and a sign of dodgy pointers or stack overflow)
    GuardPage(GuardPageType),
}
impl AllocationType {
    fn _map_into_vmem(phys_addr: usize, size: PageAllocationSizeT) -> (impl AnyPageAllocation,*mut u8) {
        // Map requested physmem into kernel space
        let vmap = KERNEL_PTABLE.allocate(size, KALLOCATION_KERNEL_GENERALDYN).unwrap();
        vmap.set_base_addr(phys_addr, pageFlags!(t:WRITEABLE,m:PINNED));
        let ptr = vmap.start().get() as *mut u8;
        // Done :)
        (vmap, ptr)
    }
    
    pub fn needs_initialisation(&self) -> bool {
        match self {
            Self::UninitMem => false,
            Self::GuardPage(_) => false,
            
            Self::ZeroedMem => true,
        }
    }
    /// SAFETY: One must ensure that `phys_addr` is an actual, page-aligned, physical address,
    ///             pointing to a minimum of `size` bytes of read-write memory
    ///             that is not in use anywhere else (noalias)
    ///             and that will remain valid and not otherwise used for the duration of this function.
    ///             (generally, holding the corresponding PhysicalMemoryAllocation in a local in the calling function is sufficient for 1,2, and 4)
    pub(self) unsafe fn initialise(&self, phys_addr: usize, size: PageAllocationSizeT) {
        if !self.needs_initialisation() { return };
        
        let (vmap, ptr) = Self::_map_into_vmem(phys_addr, size);
        // Initialise memory as requested
        match self {
            Self::UninitMem | Self::GuardPage(_) => unreachable!(),
            
            Self::ZeroedMem => {
                core::ptr::write_bytes(ptr, 0, size.get());  // zero out the memory. FIXME: ensure this doesn't get optimized out
            },
        }
        // And free the mapping now that we're done
        drop(vmap);
    }
}

/// The type of backing currently in use
enum BackingType {
    /// Physical memory (not shared)
    PhysMemExclusive(PhysicalMemoryAllocation),
    /// Physical memory (shared due to splitting or similar reasons)
    PhysMemShared { allocation: Arc<PhysicalMemoryAllocation>, offset: usize },
    
    /// Copy-on-write (DRAFT)
    CopyOnWrite(Arc<BackingSection>),
    
    /// Reserved memory - not ready yet, should be initialised on access
    ReservedMem,
}
struct BackingSection {
    mode: BackingType,
    size: PageAllocationSizeT,
}
impl BackingSection {
    /// Get the physical address, if currently in physical memory
    pub fn get_phys_addr(&self) -> Option<usize> {
        match self.mode {
            BackingType::PhysMemExclusive(ref alloc) => Some(alloc.get_addr()),
            BackingType::PhysMemShared { ref allocation, offset } => Some(allocation.get_addr()+offset),
            BackingType::CopyOnWrite(ref cow) => cow.get_phys_addr(),
            BackingType::ReservedMem => None,
        }
    }
    /// Get the size
    pub fn get_size(&self) -> PageAllocationSizeT {
        self.size
    }
    
    /// Get whether this is read-only or read-write
    /// (for unreadable types (such as reserved) -> an unspecified but valid boolean)
    pub fn is_read_only(&self) -> bool {
        const UD: bool = false;
        match self.mode {
            BackingType::PhysMemExclusive(_) | BackingType::PhysMemShared { .. } => false,
            BackingType::CopyOnWrite(_) => true,
            BackingType::ReservedMem => UD,
        }
    }
}
struct AllocationBacking {
    sections: VecDeque<BackingSection>,
    requested_type: AllocationType,
    
    offset: PageAlignedOffsetT,
    total_size: PageAllocationSizeT,
}

struct VirtualAllocation {
    allocation: Box<dyn AnyPageAllocation>,
    size: PageAllocationSizeT,
    
    absent_pages_table_handle: AbsentPagesHandleA,
}
impl VirtualAllocation {
    pub fn new(allocation: Box<dyn AnyPageAllocation>,
               meta_a: AbsentPagesItemA,
                ) -> Self {
        let size = allocation.size();
        
        let apt_initialiser = ABSENT_PAGES_TABLE.create_new_descriptor();
        apt_initialiser.slot_t().pt_phys_addr.store(allocation.pt_phys_addr(), core::sync::atomic::Ordering::Relaxed);
        apt_initialiser.slot_t().virt_addr.store(allocation.start().get(), core::sync::atomic::Ordering::Relaxed);
        let apth = apt_initialiser.commit(meta_a, AbsentPagesItemB{});
        
        let apth_a = apth.downgrade();
        Self { allocation: allocation, size: size, absent_pages_table_handle: apth_a }
    }
}
struct VirtualSlot {
    allocations: VecDeque<VirtualAllocation>,
    default_flags: PageFlags,
    
    offset: PageAlignedOffsetT,
}

struct UnifiedAllocationInner {
    virt_slots: Vec<Option<VirtualSlot>>,
    backing: AllocationBacking,
}
impl UnifiedAllocationInner {
    fn _alloc_new(btype: AllocationType, size: PageAllocationSizeT) -> Option<Self> {
        let backing_item = Self::_allocate_new_backing(btype, size)?;
        let backing = AllocationBacking {
            sections: vec![BackingSection {
                mode: backing_item,
                size: size,
            }].into(),
            requested_type: btype,
            total_size: size,
            offset: PageAlignedOffsetT::new(0),
        };
        Some(Self {
            virt_slots: vec![None],
            backing: backing,
        })
    }
}

impl UnifiedAllocationInner {  // EXPANSION/SHRINKING
    fn _allocate_new_backing(btype: AllocationType, size: PageAllocationSizeT) -> Option<BackingType> {
        match btype {
            AllocationType::UninitMem | AllocationType::ZeroedMem => {
                // RAM
                let phys_allocation = palloc(size)?;
                
                // Initialise RAM
                // SAFETY: The allocation is specified to have the given address and size, so we're good.
                unsafe {
                    let addr = phys_allocation.get_addr();
                    debug_assert!(phys_allocation.get_size() >= size);
                    btype.initialise(addr, size);
                }
                
                // Return backing
                Some(BackingType::PhysMemExclusive(phys_allocation))
            },
            AllocationType::GuardPage(gptype) => {
                // Guard Page (doesn't occupy RAM)
                Some(BackingType::ReservedMem)
            },
        }
    }
    
    /// Expand this allocation downwards (into lower logical addresses) by the given amount
    fn expand_downwards(&mut self, size: PageAllocationSizeT) -> bool {
        // Allocate the new backing
        let alloc_type = self.backing.requested_type;
        let Some(allocation) = Self::_allocate_new_backing(alloc_type, size) else {return false};
        
        // Update our backing sections
        self.backing.sections.push_front(BackingSection {
            mode: allocation,
            size: size,
        });
        self.backing.total_size = PageAllocationSizeT::new(self.backing.total_size.get() + size.get());
        self.backing.offset = add_offset_and_size!((self.backing.offset) - (size));
        true
    }
    /// Expand this allocation upwards (into higher logical addresses) by the given amount
    fn expand_upwards(&mut self, size: PageAllocationSizeT) -> bool {
        let alloc_type = self.backing.requested_type;
        let Some(allocation) = Self::_allocate_new_backing(alloc_type, size) else {return false};
        
        // Update our backing sections
        self.backing.sections.push_back(BackingSection {
            mode: allocation,
            size: size,
        });
        self.backing.total_size = PageAllocationSizeT::new(self.backing.total_size.get() + size.get());
        // (we don't have to update offset)
        true
    }
    
    /// Attempt to expand all virtual memory allocations tied to this, such that they can hold this allocation in full
    /// Returns a Vec containing the index of each slot that is now large enough to fit the whole allocation.
    fn expand_vmem(&mut self, self_arc: &Arc<UnifiedAllocationLockedInner>) -> Vec<VirtAllocSlotIndex> {
        let bottom_offset: PageAlignedOffsetT = self.backing.offset;
        let top_offset: PageAlignedOffsetT = self.backing.sections.iter().fold(self.backing.offset, |a,sec|add_offset_and_size!((a) + (sec.get_size())));
        
        // Expand vmem downwards
        let mut successful_down = Vec::with_capacity(self.virt_slots.len());
        for (virt_idx,virt_slot) in self.virt_slots.iter_mut().enumerate().filter_map(|(i,o)|Some((i,o.as_mut()?))) {
            let old_allocation = &mut virt_slot.allocations.front_mut().expect("VirtualAllocations should always have at least one section.").allocation;
            let old_offset = virt_slot.offset;
            
            let amount_missing = old_offset - bottom_offset;  // Figure out how much to expand by. If a previous downwards allocation failed, this will retry it. If there is excess, then the excess can be re-used.
            if amount_missing.get() <= 0 { successful_down.push(virt_idx); continue };  // skip if we already have enough
            let amount_missing = PageAllocationSizeT::new(amount_missing.get() as usize);
            
            let Some(new_allocation) = old_allocation.alloc_downwards_dyn(amount_missing) else{continue};  // attempt to allocate downwards
            // Success :)
            let new_allocation = VirtualAllocation::new(new_allocation, AbsentPagesItemA::new_normal(self_arc, virt_idx, bottom_offset));
            virt_slot.allocations.push_front(new_allocation);
            successful_down.push(virt_idx);
        }
        // Expand vmem upwards
        let mut successful_up = Vec::with_capacity(self.virt_slots.len());
        for (virt_idx,virt_slot) in self.virt_slots.iter_mut().enumerate().filter_map(|(i,o)|Some((i,o.as_mut()?))) {
            let old_allocation = &mut virt_slot.allocations.back_mut().expect("VirtualAllocations should always have at least one section.").allocation;
            let old_offset = virt_slot.allocations.iter().fold(virt_slot.offset, |a,alloc|add_offset_and_size!((a) + (alloc.size)));
            
            let amount_missing = top_offset - old_offset ;
            if amount_missing.get() <= 0 { successful_up.push(virt_idx); continue };
            let amount_missing = PageAllocationSizeT::new(amount_missing.get() as usize);
            
            let Some(new_allocation) = None else{continue};  // TODO
            // Success :)
            let new_allocation = VirtualAllocation::new(new_allocation, AbsentPagesItemA::new_normal(self_arc, virt_idx, old_offset));
            virt_slot.allocations.push_back(new_allocation);
            successful_up.push(virt_idx);
        }
        
        // Remap vmem to match our backing sections
        self._remap_pages(self_arc, None,false);
        // Return successful slots
        successful_down.retain(|x|successful_up.contains(x));
        return successful_down;
    }
}

impl UnifiedAllocationInner {  // PAGE REMAPPING
    fn _remap_page(backing: &AllocationBacking, section: &BackingSection,
                   slot: &VirtualSlot, virt: &VirtualAllocation, mapping_addr_offset: usize,
                   force_readonly: bool) {
        let addr: Option<usize> = section.get_phys_addr();
        match addr {
            Some(addr) => {
                let addr = addr + mapping_addr_offset;
                // Determine flags to use
                let mut flags: PageFlags = slot.default_flags;
                if force_readonly || section.is_read_only() { flags -= pageFlags!(t:WRITEABLE); }
                klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t\t Mapping -> phys={:x} flags={:?}", addr, flags);
                virt.allocation.set_base_addr(addr, flags);
            },
            None => {
                let abt_id: usize = virt.absent_pages_table_handle.get_id().try_into().unwrap();
                klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t\t Mapping -> absent (ID#{})", abt_id);
                virt.allocation.set_absent(abt_id);
            },
        }
    }
    
    /// Remap all pages (faster, but requires all section borders to be aligned with virtual allocation borders)
    /// If virt_slot is not None, only remaps that specific slot. Otherwise, remaps all occupied slots.
    /// If force_readonly is true, maps the pages as read only (even if normally they wouldn't be). Useful for avoiding race conditions when switching backings.
    fn _remap_pages_fast(&self, virt_slot: Option<usize>, force_readonly: bool) {
        let mut backing_start_offset = self.backing.offset;
        let mut backing_end_offset = backing_start_offset;
        
        struct FastVirtRemapState<'a,Iter:Iterator<Item=&'a VirtualAllocation>+'a> {
            prev_alloc_end: PageAlignedOffsetT,
            to_process: Iter,
            slot: &'a VirtualSlot,
            
            log_idx: usize,
        }
        let mut virt_states: Vec<Option<_>> = self.virt_slots.iter()
            .enumerate().filter(|(i,x)| virt_slot.is_none() || *i==virt_slot.unwrap())  // filter by virt_slot
            .filter_map(|(i,o)|o.as_ref().map(|x|(i,x))).map(|(i,slot)|Some(FastVirtRemapState{
                prev_alloc_end: slot.offset,
                to_process: slot.allocations.iter(),
                slot: slot,
                log_idx: i,
            })).collect();
        
        for (log_sec_idx, backing_item) in self.backing.sections.iter().enumerate() {
            backing_start_offset = backing_end_offset;
            backing_end_offset = add_offset_and_size!((backing_start_offset) + (backing_item.size));
            klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "Remapping(fast) section#{} - off_start={:x} off_end={:x} size={:x}", log_sec_idx, backing_start_offset, backing_end_offset, backing_item.size);
            
            // Handle each slot of virtual allocations
            for virt_o in virt_states.iter_mut() {
                let Some(virt) = virt_o.as_mut() else {continue};
                klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t virt_state#{}", virt.log_idx);
                loop { // for each allocation
                    let Some(virt_alloc) = virt.to_process.next() else {
                        let _ = virt;
                        *virt_o = None;
                        break;
                    };
                    let virt_start_offset = virt.prev_alloc_end;
                    let virt_end_offset = add_offset_and_size!((virt_start_offset) + (virt_alloc.size)); virt.prev_alloc_end = virt_end_offset;
                    klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t off_start={:x} off_end={:x} size={:x}", virt_start_offset, virt_end_offset, virt_alloc.size);
                    
                    // Check we're not past it
                    if virt_start_offset >= backing_end_offset { break; }  // Not for this section (but for the next). We're done
                    // Preconditions for using _fast instead of _remap_pages (checked in debug mode only)
                    debug_assert!(virt_start_offset >= backing_start_offset);
                    debug_assert!(virt_end_offset <= backing_end_offset);
                    // Remap it
                    let addr_offset: isize = (virt_start_offset-backing_start_offset).into();
                    let addr_offset: usize = addr_offset.try_into().unwrap();
                    klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t Mapping with addr_offset={:x} force_readonly={}", addr_offset, force_readonly);
                    Self::_remap_page(&self.backing, backing_item, virt.slot, virt_alloc, addr_offset, force_readonly);
                    // Done
                }  // NEXT virt_alloc
            }  // NEXT virt_o
        } // NEXT backing_item
    }
    
    /// Remap all pages to point to the newly modified backing
    /// If virt_slot is not None, only remaps that specific slot. Otherwise, remaps all occupied slots.
    fn _remap_pages(&mut self, self_arc: &Arc<UnifiedAllocationLockedInner>,
                    virt_slot: Option<usize>, force_readonly: bool) {
        let mut backing_processed = 0usize; let mut backing_remaining = self.backing.sections.len();
        let mut backing_start_offset = self.backing.offset;
        let mut backing_end_offset = backing_start_offset;
        
        struct VirtRemapState<'a> {
            prev_alloc_end: PageAlignedOffsetT,
            to_process: VecDeque<VirtualAllocation>,
            slot: &'a mut VirtualSlot,
            slot_idx: VirtAllocSlotIndex,
        }
        let mut virt_states: Vec<Option<_>> = self.virt_slots.iter_mut()
            .enumerate().filter(|(i,x)| virt_slot.is_none() || *i==virt_slot.unwrap())  // filter by virt_slot
            .filter_map(|(i,o)|o.as_mut().map(|x|(i,x))).map(|(i,slot)|Some(VirtRemapState{
                prev_alloc_end: slot.offset,
                to_process: slot.allocations.drain(0..).collect(),
                slot: slot,
                slot_idx: i,
            })).collect();
        
        for (log_sec_idx, backing_item) in self.backing.sections.iter().enumerate() {
            // Re-calculate backing offsets
            backing_start_offset = backing_end_offset;
            backing_end_offset = add_offset_and_size!((backing_start_offset) + (backing_item.size));
            let is_first_section = backing_processed==0; backing_processed += 1;
            backing_remaining -= 1; let is_last_section = backing_remaining==0;
            klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "Remapping section#{} - off_start={:x} off_end={:x} size={:x} [first={}/last={}/prc={}/rem={}]", log_sec_idx, backing_start_offset, backing_end_offset, backing_item.size, is_first_section, is_last_section, backing_processed, backing_remaining);
            
            // Now, handle each set of virtual allocations
            for virt_o in virt_states.iter_mut() {  // for each slot
                let Some(virt) = virt_o.as_mut() else {continue};
                klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t virt_state#{}", virt.slot_idx);
                loop {  // for each allocation - we break once we're ready to process the next slot
                    // Take next allocation from queue
                    let Some(virt_alloc) = virt.to_process.pop_front() else {
                        // We're done
                        let _ = virt;
                        *virt_o = None;
                        break;  // break out of the loop and process the next slot
                    };
                    let virt_start_offset = virt.prev_alloc_end;
                    let virt_end_offset = add_offset_and_size!((virt_start_offset) + (virt_alloc.size)); virt.prev_alloc_end = virt_end_offset;
                    klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t off_start={:x} off_end={:x} size={:x}", virt_start_offset, virt_end_offset, virt_alloc.size);
                    
                    // And process it
                    if virt_end_offset <= backing_start_offset {
                        // We're already past this allocation's backing
                        klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t Before current section. Dropping.");
                        if is_first_section {
                            // If we're past it and is_first_section, then it's no longer part of this allocation and should be dropped
                            drop(virt_alloc);
                            continue;
                        } else {
                            // This should've already been processed
                            unreachable!()
                        }
                    } else if virt_start_offset >= backing_end_offset {
                        // It's after us
                        // We've processed everything we can for the moment
                        virt.to_process.push_front(virt_alloc);
                        break;
                    } else if virt_start_offset >= backing_start_offset && virt_end_offset <= backing_end_offset {
                        // Good news, everyone
                        // It's entirely within us, so we simply have to map it
                        let addr_offset: isize = (virt_start_offset-backing_start_offset).into();  // offset in memory against the backing
                        let addr_offset: usize = addr_offset.try_into().unwrap();
                    klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t Mapping with addr_offset={:x} force_readonly={}", addr_offset, force_readonly);
                        Self::_remap_page(&self.backing, backing_item, &*virt.slot, &virt_alloc, addr_offset, force_readonly);
                        // And then push it
                        virt.slot.allocations.push_back(virt_alloc);
                        continue;
                    } else if virt_end_offset > backing_end_offset {
                        // It goes beyond our end
                        let part_of_us_size: isize = (backing_end_offset - virt_start_offset).into();  // find the size of the part that's inside us
                        let part_of_us_size: usize = part_of_us_size.try_into().unwrap();
                        let part_of_us_size: PageAllocationSizeT = part_of_us_size.try_into().unwrap();
                        klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t Overlaps end. Splitting with lhs_size={:x}", part_of_us_size);
                        // Split it
                        let VirtualAllocation { allocation: old_alloc, .. } = virt_alloc;
                        let (lhs, rhs) = old_alloc.split_dyn(part_of_us_size);
                        // Push rhs back to be processed later
                        let rhs: VirtualAllocation = VirtualAllocation::new(rhs, AbsentPagesItemA::new_normal(self_arc, virt.slot_idx, backing_end_offset));
                        virt.to_process.push_front(rhs);
                        // Push lhs back to be processed momentarily
                        let lhs: VirtualAllocation = VirtualAllocation::new(lhs, AbsentPagesItemA::new_normal(self_arc, virt.slot_idx, virt_start_offset));
                        virt.to_process.push_front(lhs);
                        continue;
                    } else if virt_start_offset < backing_start_offset {
                        // It overlaps our start
                        let start_overran_by: isize = (virt_start_offset - backing_start_offset).into();
                        let start_overran_by: usize = start_overran_by.try_into().unwrap();
                        let start_overran_by: PageAllocationSizeT = start_overran_by.try_into().unwrap();
                        klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t\t\t Overlaps start. Splitting with lhs_size={:x}", start_overran_by);
                        // Split it
                        let VirtualAllocation { allocation: old_alloc, .. } = virt_alloc;
                        let (lhs, rhs) = old_alloc.split_dyn(start_overran_by);
                        // Push rhs back to be processed later
                        let rhs: VirtualAllocation = VirtualAllocation::new(rhs, AbsentPagesItemA::new_normal(self_arc, virt.slot_idx, backing_start_offset));
                        virt.to_process.push_front(rhs);
                        // The lhs must either be out-of-bounds (no longer needed), or we've missed it
                        if is_first_section { drop(lhs) }
                        else { unreachable!() }
                        // Continue processing
                        continue;
                    } else {
                        unreachable!()
                    }
                    // We use continue explicitly in the if statement above to avoid accidentally "falling through" without having properly handled the allocation (pushing it back if needed)
                    // Thus, this line is unreachable. If it is not unreachable, that is an error.
                    #[deny(unfulfilled_lint_expectations)]
                    #[expect(unreachable_code, reason="Each branch should choose explicitly to either `continue` (process the next virtual allocation) or `break` (process the next virtual slots + backing section).")]
                    { unreachable!() }
                }
            } // NEXT virt_o (virt slot)
        } // NEXT backing_item (backing section)
        
        // All done.
        // Any virtual memory allocations which were not re-mapped are no longer part of the allocation, and will be dropped at the end of this function
    }
}

impl UnifiedAllocationInner {  // VIRT SLOT ADDING/CLEARING
    /// Returns the index of the slot used
    /// If allocation.size is < self.size, this might only map part of the allocation. Changing offset to a value above zero allows you to offset the mapped area deeper into the unified allocation. (start of virt allocation = start of backing + offset)
    fn _new_virt_mapping(&mut self, self_arc: &Arc<UnifiedAllocationLockedInner>,
                         allocation: Box<dyn AnyPageAllocation>, flags: PageFlags, offset: PageAlignedOffsetT) -> VirtAllocSlotIndex {
        let (slot_idx, slot_mut) = match self.virt_slots.iter().position(Option::is_none) {
            Some(slot_idx) => {
                let slot_mut = &mut self.virt_slots[slot_idx];
                (slot_idx, slot_mut)
            },
            None => {
                let slot_idx = self.virt_slots.len();
                self.virt_slots.push(None);
                let slot_mut = &mut self.virt_slots[slot_idx];
                (slot_idx, slot_mut)
            },
        };
        
        // Initialise the slot
        let offset = self.backing.offset + offset;
        *slot_mut = Some(VirtualSlot {
            allocations: vec![VirtualAllocation::new(allocation,AbsentPagesItemA::new_normal(self_arc, slot_idx, offset))].into(),
            default_flags: flags,
            offset: offset,
        });
        // And remap it (this will break the allocation into the necessary pieces as well)
        self._remap_pages(self_arc, Some(slot_idx), false);
        
        // And return
        slot_idx
    }
    /// Clear the given slot
    fn _clear_virt_mapping(&mut self, slot: VirtAllocSlotIndex) {
        let prev_value = self.virt_slots[slot].take();
        debug_assert!(prev_value.is_some(), "Cleared virt stot that was already empty!");
        drop(prev_value);  // dropping the prev value will now clear the page mappings it previously held
    }
}

type UnifiedAllocationLockedInner = YMutex<UnifiedAllocationInner>;
pub struct UnifiedAllocation(Arc<UnifiedAllocationLockedInner>);
impl UnifiedAllocation {
    pub fn alloc_new(btype: AllocationType, size: PageAllocationSizeT) -> Option<Self> {
        Some(Self(Arc::new(YMutex::new(UnifiedAllocationInner::_alloc_new(btype, size)?))))
    }
    pub fn clone_ref(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
    
    /// Get the current total size of this allocation
    pub fn size(&self) -> PageAllocationSizeT {
        self.0.lock().backing.total_size
    }
    
    /// Expand downwards, returning the index of every virtual allocation that was successfully resized
    pub fn expand_downwards(&self, size: PageAllocationSizeT) -> Vec<VirtAllocSlotIndex> {
        let mut inner = self.0.lock();
        inner.expand_downwards(size);
        let successful_virts = inner.expand_vmem(&self.0);
        // (pages are remapped by expand_vmem)
        return successful_virts;
    }
    /// Expand upwards, returning the index of every virtual allocation that was successfully resized
    pub fn expand_upwards(&self, size: PageAllocationSizeT) -> Vec<VirtAllocSlotIndex> {
        let mut inner = self.0.lock();
        inner.expand_upwards(size);
        let successful_virts = inner.expand_vmem(&self.0);
        // (pages are remapped by expand_vmem)
        return successful_virts;
    }
    
    /// Map a vmem allocation to point to this allocation.
    /// If virt_allocation.size >= self.size, then offset should be zero. If virt_allocation.size < self.size, then offset may be set to a number to decide which part of this allocation gets mapped.
    /// This returns an allocation guard. When the guard is dropped, the pages are unmapped from virtual memory.
    ///
    /// NOTE: You MUST NOT hold a lock on the PageAllocator you got the allocation from when calling this method, as that will cause a deadlock.
    ///         All paging-related methods in this class assume that no PageAllocator locks are held in the current thread (and the lock internal to this allocation is always taken before it locks any pageallocators)
    pub fn map_vmem(&self, virt_allocation: Box<dyn AnyPageAllocation>, flags: PageFlags, offset: PageAlignedOffsetT) -> UnifiedVirtGuard {
        let slot = self.0.lock()._new_virt_mapping(&self.0, virt_allocation, flags, offset);
        // page is mapped by _new_virt_mapping
        return UnifiedVirtGuard { alloc: self.clone_ref(), slot_index: slot };
    }
}
pub type VirtAllocSlotIndex = usize;

pub struct UnifiedVirtGuard {
    alloc: UnifiedAllocation,
    slot_index: VirtAllocSlotIndex,
}
impl UnifiedVirtGuard {
    pub fn get_alloc(&self) -> &UnifiedAllocation {
        &self.alloc
    }
    pub fn get_slot_index(&self) -> VirtAllocSlotIndex {
        self.slot_index
    }

    pub(self) fn slot_mut(&self) -> MappedYMutexGuard<VirtualSlot> {
        YMutexGuard::map(self.alloc.0.lock(), |inner|
            inner.virt_slots[self.slot_index].as_mut().unwrap()
        )
    }
    
    /// Get the start and total size of this allocation in vmem.
    pub fn get_bounds(&self) -> (PageAlignedAddressT,PageAllocationSizeT) {
        let inner = self.alloc.0.lock();
        let slot = inner.virt_slots[self.slot_index].as_ref().unwrap();
        
        let start = slot.allocations.front().unwrap().allocation.start();
        let size = PageAllocationSizeT::new(slot.allocations.iter().fold(0,|a,x|a+x.size.get()));
        (start, size)
    }
}
impl Drop for UnifiedVirtGuard {
    fn drop(&mut self) {
        self.alloc.0.lock()._clear_virt_mapping(self.slot_index)
    }
}

use crate::descriptors::{DescriptorTable, DescriptorHandleA, DescriptorHandleB, DescriptorID};
use crate::memory::alloc_util::AnyAllocatedStack;

#[derive(Default)]
struct AbsentPagesItemT {
    // These two values can be used to aid in locating allocations if a direct Descriptor ID is not provided
    /// Physical address of the page table the allocation was created in
    pt_phys_addr: core::sync::atomic::AtomicUsize,
    /// Virtual address of the allocation (in vmem)
    virt_addr: core::sync::atomic::AtomicUsize,
}
enum AbsentPagesItemA {
    NormalAllocation {
        allocation: Weak<UnifiedAllocationLockedInner>,
        virt_slot: VirtAllocSlotIndex,
        offset: PageAlignedOffsetT,
    },
    /// Used when no allocation corresponds to the given page any more (i.e. it's been leaked),
    ///  and the allocation in question is a guard page.
    /// Since no Unified Allocation remains, the page will remain a guard page until its page table
    ///  is dropped. (at which point it no longer exists)
    /// (Saves keeping a spare allocation lying around for every null guard)
    StaticGuardPage(GuardPageType),
}
impl AbsentPagesItemA {
    pub fn new_normal(allocation: &Arc<UnifiedAllocationLockedInner>, virt_slot: VirtAllocSlotIndex, offset: PageAlignedOffsetT) -> Self {
        Self::NormalAllocation {
            allocation: Arc::downgrade(allocation),
            virt_slot, offset,
        }
    }
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();

    pub static ref ABSENT_PAGES_ID_NULL_GUARD: DescriptorID = {
        let initializer = ABSENT_PAGES_TABLE.create_new_descriptor();
        let handle = initializer.commit(
            AbsentPagesItemA::StaticGuardPage(GuardPageType::NullPointer),
            AbsentPagesItemB{}
        );
        let id = handle.get_id();
        core::mem::forget(handle);  // leak the handle so its ID is valid forever
        id
    };
}

// == SPECIALISED ALLOCATIONS ==
/// A special allocation - represents a stack which can be expanded.
/// Stacks are privately owned by their respective vmem allocations.
/// (assumes stack grows downwards)
pub struct AllocatedStack {
    stack_main: UnifiedVirtGuard,
    guard_page: UnifiedVirtGuard,

    page_flags: PageFlags,
    guard_size: PageAllocationSizeT,
}
impl AllocatedStack {
    pub fn alloc_new<PFA:PageFrameAllocator+Send+Sync+'static>(
        stack_size: PageAllocationSizeT, guard_size: PageAllocationSizeT,
        allocator: &LockedPageAllocator<PFA>, strategy: PageAllocationStrategies, page_flags: PageFlags,
    ) -> Option<Self> {
        let main_alloc = UnifiedAllocation::alloc_new(AllocationType::UninitMem,stack_size)?;
        let guard_alloc = UnifiedAllocation::alloc_new(AllocationType::GuardPage(GuardPageType::StackLimit),guard_size)?;

        let size_total = PageAllocationSizeT::new(stack_size.get() + guard_size.get());
        let virt_total = allocator.allocate(size_total, strategy)?;
        let (virt_guard, virt_main) = virt_total.split(guard_size);
        let virt_guard = Box::new(virt_guard); let virt_main = Box::new(virt_main);

        let vu_guard = guard_alloc.map_vmem(virt_guard, page_flags, PageAlignedOffsetT::new(0));
        let vu_main = main_alloc.map_vmem(virt_main, page_flags, PageAlignedOffsetT::new(0));

        Some(Self {
            stack_main: vu_main,
            guard_page: vu_guard,
            guard_size, page_flags,
        })
    }

    /// Get the bottom of the stack
    pub fn bottom_vaddr(&self) -> PageAlignedAddressT {
        let (max_top, size) = self.stack_main.get_bounds();
        PageAlignedAddressT::new(max_top.get() + size.get())
    }

    pub fn expand(&mut self, amount: PageAllocationSizeT) -> bool {
        let extra_amount = PageAllocationSizeT::new_checked(amount.get()-self.guard_size.get());
        let total_to_allocate = PageAllocationSizeT::new(extra_amount.unwrap_or(PageAllocationSizeT::new(0)).get() + self.guard_size.get());

        // Check we can allocate what we need
        let Some(new_alloc) = self.guard_page.slot_mut().allocations[0].allocation.alloc_downwards_dyn(total_to_allocate) else {
            return false;  // we failed - we can return without having to put anything back
        };
        // And split into "new guard page" and "extra main beyond the previous guard page"
        // (we re-use the previous guard page's virt allocation for efficiency's sake)
        let (new_guard_alloc, extra_main_alloc) = if let Some(extra_amount) = extra_amount {
            let (lhs, rhs) = new_alloc.split_dyn(self.guard_size);
            (lhs, Some(rhs))
        } else {
            (new_alloc, None)
        };

        // Move old guard page allocation (+ extra main) over to start of main allocation
        // (this method isn't exposed publically because only we can guarantee that these two allocations
        //  are back-to-back in virtual memory)
        {
            let guard_virt_allocs: Vec<_> = self.guard_page.slot_mut().allocations.drain(..).collect();
            let mut main_slot_mut = self.stack_main.slot_mut();
            for gva in guard_virt_allocs.into_iter().rev() {
                let new_offset = add_offset_and_size!((main_slot_mut.offset) - (gva.size));
                main_slot_mut.offset = new_offset;  // update start offset
                let gva = VirtualAllocation::new(gva.allocation, AbsentPagesItemA::new_normal(&self.stack_main.alloc.0, self.stack_main.slot_index, new_offset));  // update the absent pages table
                main_slot_mut.allocations.push_front(gva);  // add allocation to the start
            }
            if let Some(extra_main_virt) = extra_main_alloc {
                let new_offset = add_offset_and_size!((main_slot_mut.offset) - (extra_main_virt.size()));
                main_slot_mut.offset = new_offset;
                let eva = VirtualAllocation::new(extra_main_virt, AbsentPagesItemA::new_normal(&self.stack_main.alloc.0, self.stack_main.slot_index, new_offset));
                main_slot_mut.allocations.push_front(eva);
            }
        }
        let expansion_results = self.stack_main.alloc.expand_downwards(amount);
        debug_assert!(expansion_results.contains(&self.stack_main.slot_index));  // this should always succeed as we literally just pushed the extra allocations necessary

        // Map new guard page
        self.guard_page = self.guard_page.alloc.map_vmem(new_guard_alloc, self.page_flags, PageAlignedOffsetT::new(0));

        // Done :)
        true
    }
}
impl Debug for AllocatedStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "AllocatedStack[bottom={:x},size={:x}]", self.bottom_vaddr(), self.stack_main.get_bounds().1.get())
    }
}
impl AnyAllocatedStack for AllocatedStack {
    fn bottom_vaddr(&self) -> usize {
        self.bottom_vaddr().get()
    }

    fn expand(&mut self, bytes: usize) -> bool {
        self.expand(PageAllocationSizeT::new_rounded(bytes))
    }
}

/// A special allocation, that is offset-mapped into the kernel data page table.
/// Unlike UnifiedAllocations, this cannot be resized or swapped out, nor can it contain guard pages or similar utilities.
/// It also cannot be shared, it is always mapped into the global kernel data page table.
pub struct OffsetMappedAllocation {
    virt: super::paging::global_pages::GlobalPageAllocation,  // virt before phys to ensure page table gets cleared first
    phys: PhysicalMemoryAllocation,
}
impl OffsetMappedAllocation {
    pub fn alloc_new(size: PageAllocationSizeT, flags: PageFlags) -> Option<Self> {
        let phys = palloc(size)?;
        let virt_addr = PageAlignedAddressT::new(phys.get_addr() + super::paging::global_pages::KERNEL_PTABLE_VADDR);
        let virt = KERNEL_PTABLE.allocate_at(virt_addr, phys.get_size());  // (allocated amount may be larger than requested due to allocator limitations)
        debug_assert!(virt.is_some(), "VMem offset-mapped allocation failed? This should never be possible.");
        let virt = virt?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { phys, virt })
    }
    
    pub fn get_phys_addr(&self) -> PageAlignedAddressT {
        PageAlignedAddressT::new(self.phys.get_addr())
    }
    pub fn get_virt_addr(&self) -> PageAlignedAddressT {
        self.virt.start()
    }
    pub fn get_size(&self) -> PageAllocationSizeT {
        debug_assert!(self.phys.get_size() == self.virt.size());
        self.phys.get_size()
    }

    /// Leak this allocation, returning its virt ptr, phys addr, and size
    /// Unlike `forget()`, this correctly drops the Arc<> for the page table,
    ///  preventing the page table itself from being leaked
    pub fn leak(self) -> (*mut u8, PageAlignedAddressT, PageAllocationSizeT) {
        let phys_addr = self.get_phys_addr();
        let virt_addr = self.get_virt_addr().get() as *mut u8;
        let size = self.get_size();
        let Self{phys, virt} = self;
        core::mem::forget(phys); virt.leak();
        (virt_addr, phys_addr, size)
    }
}