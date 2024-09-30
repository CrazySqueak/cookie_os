
use core::ops::{Deref,DerefMut};
use alloc::boxed::Box;
use alloc::vec::Vec; use alloc::vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,PageAlignedValue,PageAllocationSizeT,PageAlignedOffsetT,PageAlignedAddressT,PageFlags,pageFlags};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::{YMutex,YMutexGuard,ArcYMutexGuard};
use crate::logging::klog;

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

macro_rules! vec_of_non_clone {
    [$item:expr ; $count:expr] => {
        Vec::from_iter((0..$count).map(|_|$item))
    }
}
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
        vmap.set_base_addr(phys_addr, pageFlags!(m:PINNED));
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
}

struct VirtualAllocation {
    allocation: Box<dyn AnyPageAllocation>,
    size: PageAllocationSizeT,
    
    absent_pages_table_handle: AbsentPagesHandleA,
}
impl VirtualAllocation {
    pub fn new(allocation: Box<dyn AnyPageAllocation>,
                meta_offset: PageAlignedOffsetT,
                ) -> Self {
        let size = allocation.size();
        
        let apt_initialiser = ABSENT_PAGES_TABLE.create_new_descriptor();
        apt_initialiser.slot_t().pt_phys_addr.store(allocation.pt_phys_addr(), core::sync::atomic::Ordering::Relaxed);
        apt_initialiser.slot_t().virt_addr.store(allocation.start().get(), core::sync::atomic::Ordering::Relaxed);
        let apth = apt_initialiser.commit(AbsentPagesItemA {
            offset: meta_offset,
        }, AbsentPagesItemB{});
        
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
    fn _alloc_new(btype: AllocationType, size: PageAllocationSizeT) -> Self {
        let backing_item = Self::_allocate_new_backing(btype, size);
        let backing = AllocationBacking {
            sections: vec![BackingSection {
                mode: backing_item,
                size: size,
            }].into(),
            requested_type: btype,
            offset: PageAlignedOffsetT::new(0),
        };
        Self {
            virt_slots: vec![None],
            backing: backing,
        }
    }
}

impl UnifiedAllocationInner {  // EXPANSION/SHRINKING
    fn _allocate_new_backing(btype: AllocationType, size: PageAllocationSizeT) -> BackingType {
        match btype {
            AllocationType::UninitMem | AllocationType::ZeroedMem => {
                // RAM
                let Some(phys_allocation) = palloc(size) else {
                    // (reserved mem - must be reallocated later when more RAM is free)
                    return BackingType::ReservedMem;
                };
                
                // Initialise RAM
                // SAFETY: The allocation is specified to have the given address and size, so we're good.
                unsafe {
                    let addr = phys_allocation.get_addr();
                    debug_assert!(phys_allocation.get_size() >= size);
                    btype.initialise(addr, size);
                }
                
                // Return backing
                BackingType::PhysMemExclusive(phys_allocation)
            },
            AllocationType::GuardPage(gptype) => {
                // Guard Page (doesn't occupy RAM)
                BackingType::ReservedMem
            },
        }
    }
    
    /// Expand this allocation downwards (into lower logical addresses) by the given amount
    fn expand_downwards(&mut self, size: PageAllocationSizeT) {
        // Allocate the new backing
        let alloc_type = self.backing.requested_type;
        let allocation = Self::_allocate_new_backing(alloc_type, size);
        
        // Update our backing sections
        self.backing.sections.push_front(BackingSection {
            mode: allocation,
            size: size,
        });
        self.backing.offset = add_offset_and_size!((self.backing.offset) - (size));
    }
    /// Expand this allocation upwards (into higher logical addresses) by the given amount
    fn expand_upwards(&mut self, size: PageAllocationSizeT) {
        let alloc_type = self.backing.requested_type;
        let allocation = Self::_allocate_new_backing(alloc_type, size);
        
        // Update our backing sections
        self.backing.sections.push_back(BackingSection {
            mode: allocation,
            size: size,
        });
        // (we don't have to update offset)
    }
    
    /// Attempt to expand all virtual memory allocations tied to this, such that they can hold this allocation in full
    /// Returns a Vec containing the index of each slot that is now large enough to fit the whole allocation.
    fn expand_vmem(&mut self) -> Vec<usize> {
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
            let new_allocation = VirtualAllocation::new(new_allocation, bottom_offset);
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
            let new_allocation = VirtualAllocation::new(new_allocation, old_offset);
            virt_slot.allocations.push_back(new_allocation);
            successful_up.push(virt_idx);
        }
        
        // Remap vmem to match our backing sections
        self._remap_pages(None,false);
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
                
                virt.allocation.set_base_addr(addr, flags);
            },
            None => {
                let abt_id: usize = virt.absent_pages_table_handle.get_id().try_into().unwrap();
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
    fn _remap_pages(&mut self, virt_slot: Option<usize>, force_readonly: bool) {
        let mut backing_processed = 0usize; let mut backing_remaining = self.backing.sections.len();
        let mut backing_start_offset = self.backing.offset;
        let mut backing_end_offset = backing_start_offset;
        
        struct VirtRemapState<'a> {
            prev_alloc_end: PageAlignedOffsetT,
            to_process: VecDeque<VirtualAllocation>,
            slot: &'a mut VirtualSlot,
            log_idx: usize,
        }
        let mut virt_states: Vec<Option<_>> = self.virt_slots.iter_mut()
            .enumerate().filter(|(i,x)| virt_slot.is_none() || *i==virt_slot.unwrap())  // filter by virt_slot
            .filter_map(|(i,o)|o.as_mut().map(|x|(i,x))).map(|(i,slot)|Some(VirtRemapState{
                prev_alloc_end: slot.offset,
                to_process: slot.allocations.drain(0..).collect(),
                slot: slot,
                log_idx: i,
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
                klog!(Debug, MEMORY_UNIFIED_PAGEMAPPING, "\t virt_state#{}", virt.log_idx);
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
                        let rhs: VirtualAllocation = VirtualAllocation::new(rhs, backing_end_offset);
                        virt.to_process.push_front(rhs);
                        // Push lhs back to be processed momentarily
                        let lhs: VirtualAllocation = VirtualAllocation::new(lhs, virt_start_offset);
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
                        let rhs: VirtualAllocation = VirtualAllocation::new(rhs, backing_start_offset);
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
    fn _new_virt_mapping(&mut self, allocation: Box<dyn AnyPageAllocation>, flags: PageFlags) -> usize {
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
        let offset = self.backing.offset;  // if allocation.size == self.backing.size (which it should be, then taking offset == backing.offset should line things up correctly
        *slot_mut = Some(VirtualSlot {
            allocations: vec![VirtualAllocation::new(allocation,offset)].into(),
            default_flags: flags,
            offset: offset,
        });
        // And remap it (this will break the allocation into the necessary pieces as well)
        self._remap_pages(Some(slot_idx), false);
        
        // And return
        slot_idx
    }
    /// Clear the given slot
    fn _clear_virt_mapping(&mut self, slot: usize) {
        let prev_value = self.virt_slots[slot].take();
        debug_assert!(prev_value.is_some(), "Cleared virt stot that was already empty!");
        drop(prev_value);  // dropping the prev value will now clear the page mappings it previously held
    }
}
// TODO

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
#[derive(Default)]
struct AbsentPagesItemT {
    // These two values can be used to aid in locating allocations if a direct Descriptor ID is not provided
    /// Physical address of the page table the allocation was created in
    pt_phys_addr: core::sync::atomic::AtomicUsize,
    /// Virtual address of the allocation (in vmem)
    virt_addr: core::sync::atomic::AtomicUsize,
}
struct AbsentPagesItemA {
    // TODO
    /// Offset within the unified allocation
    offset: PageAlignedOffsetT,
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,AbsentPagesItemT,AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();
}

/// A special allocation, that is offset-mapped into the kernel data page table.
/// Unlike UnifiedAllocations, this cannot be resized or swapped out, nor can it contain guard pages or similar utilities.
/// It also cannot be shared, it is always mapped into the global kernel data page table.
struct OffsetMappedAllocation {
    virt: super::paging::global_pages::GlobalPageAllocation,  // virt before phys to ensure page table gets cleared first
    phys: PhysicalMemoryAllocation,
}
impl OffsetMappedAllocation {
    pub fn alloc_new(size: PageAllocationSizeT, flags: PageFlags) -> Option<Self> {
        let phys = palloc(size)?;
        let virt_addr = PageAlignedAddressT::new(phys.get_addr() + super::paging::global_pages::KERNEL_PTABLE_VADDR);
        let virt = KERNEL_PTABLE.allocate_at(virt_addr, size);
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
}