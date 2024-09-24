
use core::ops::{Deref,DerefMut};
use alloc::boxed::Box;
use alloc::vec::Vec; use alloc::vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,PageAlignedValue,PageAllocationSizeT,PageAlignedOffsetT,PageFlags};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::{YMutex,YMutexGuard,ArcYMutexGuard};

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
    }
}

#[derive(Clone,Copy,Debug)]
pub enum GuardPageType {
    StackLimit = 0xF47B33F,  // Fat Beef
    NullPointer = 0x4E55_4C505452,  // "NULPTR"
}

// TODO

struct BackingSection {
    // TODO
    
    size: PageAllocationSizeT,
}
struct AllocationBacking {
    sections: VecDeque<BackingSection>,
    
    offset: PageAlignedOffsetT,
}

struct VirtualAllocation {
    allocation: Box<dyn AnyPageAllocation>,
    size: PageAllocationSizeT,
    
    absent_pages_table_handle: AbsentPagesHandleA,
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
    fn _remap_page(backing: &AllocationBacking, section: &BackingSection,
                   slot: &VirtualSlot, virt: &VirtualAllocation, mapping_addr_offset: usize) {
        let addr: Option<usize> = None;  // TODO
        match addr {
            Some(addr) => {
                let addr = addr + mapping_addr_offset;
                let flags: PageFlags = slot.default_flags;  // TODO: adjust flags if needed based on conditions
                virt.allocation.set_base_addr(addr, flags);
            },
            None => {
                let abt_id: usize = virt.absent_pages_table_handle.get_id().try_into().unwrap();
                virt.allocation.set_absent(abt_id);
            },
        }
    }
    
    /// Remap all pages to point to the newly modified backing
    fn _remap_pages(&mut self) {
        let mut backing_processed = 0usize; let mut backing_remaining = self.backing.sections.len();
        let mut backing_start_offset = self.backing.offset;
        let mut backing_end_offset = backing_start_offset;
        
        struct VirtRemapState<'a> {
            prev_alloc_end: PageAlignedOffsetT,
            to_process: VecDeque<VirtualAllocation>,
            slot: &'a mut VirtualSlot,
        }
        let mut virt_states: Vec<Option<_>> = self.virt_slots.iter_mut().flatten().map(|slot|Some(VirtRemapState{
            prev_alloc_end: slot.offset,
            to_process: slot.allocations.drain(0..).collect(),
            slot: slot,
        })).collect();
        
        for backing_item in self.backing.sections.iter() {
            // Re-calculate backing offsets
            backing_start_offset = backing_end_offset;
            backing_end_offset = add_offset_and_size!((backing_start_offset) + (backing_item.size));
            let is_first_section = backing_processed==0; backing_processed += 1;
            backing_remaining -= 1; let is_last_section = backing_remaining==0;
            
            // Now, handle each set of virtual allocations
            for virt_o in virt_states.iter_mut() {  // for each slot
                let Some(virt) = virt_o.as_mut() else {continue};
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
                    
                    // And process it
                    if virt_end_offset <= backing_start_offset {
                        // We're already past this allocation's backing
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
                        Self::_remap_page(&self.backing, backing_item, &*virt.slot, &virt_alloc, addr_offset);
                        // And then push it
                        virt.slot.allocations.push_back(virt_alloc);
                        continue;
                    } else if virt_end_offset > backing_end_offset {
                        // It goes beyond our end
                        let part_of_us_size: isize = (backing_end_offset - virt_start_offset).into();  // find the size of the part that's inside us
                        let part_of_us_size: usize = part_of_us_size.try_into().unwrap();
                        let part_of_us_size: PageAllocationSizeT = part_of_us_size.try_into().unwrap();
                        // Split it
                        let VirtualAllocation { allocation: old_alloc, .. } = virt_alloc;
                        let (lhs, rhs) = old_alloc.split_dyn(part_of_us_size);
                        // Push rhs back to be processed later
                        let rhs: VirtualAllocation = todo!();
                        virt.to_process.push_front(rhs);
                        // Push lhs back to be processed momentarily
                        let lhs: VirtualAllocation = todo!();
                        virt.to_process.push_front(lhs);
                        continue;
                    } else if virt_start_offset < backing_start_offset {
                        // It overlaps our start
                        let start_overran_by: isize = (virt_start_offset - backing_start_offset).into();
                        let start_overran_by: usize = start_overran_by.try_into().unwrap();
                        let start_overran_by: PageAllocationSizeT = start_overran_by.try_into().unwrap();
                        // Split it
                        let VirtualAllocation { allocation: old_alloc, .. } = virt_alloc;
                        let (lhs, rhs) = old_alloc.split_dyn(start_overran_by);
                        // Push rhs back to be processed later
                        let rhs: VirtualAllocation = todo!();
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

// TODO

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
struct AbsentPagesItemA {
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<(),AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();
}