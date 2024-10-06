#![allow(deprecated)]
use crate::logging::klog;
use core::alloc::Layout;
use alloc::vec::Vec; use alloc::vec;
use alloc::boxed::Box;
use core::fmt::{Debug, Formatter};
use crate::memory::physical::{PhysicalMemoryAllocation, palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK, PageFlags, TransitivePageFlags, MappingSpecificPageFlags, PageFrameAllocator, PageAllocation, TLPageFrameAllocator, LockedPageAllocator, PageAllocationStrategies, ALLOCATION_USER_STACK, PagingContext, AnyPageAllocation, PageAlignedValue, PageAllocationSizeT as PageAlignedUsize, PageAlignedAddressT, PageAllocationSizeT, pageFlags, PageAlignedOffsetT};
use crate::memory::paging::global_pages::{GPageFrameAllocator,KERNEL_PTABLE};
use crate::memory::unified;
use crate::memory::unified::{AllocationType, GuardPageType, OffsetMappedAllocation, UnifiedAllocation, UnifiedVirtGuard};

/* Any allocated stack, regardless of how it was allocated */
pub trait AnyAllocatedStack: Debug + Send {
    fn bottom_vaddr(&self) -> usize;
    fn expand(&mut self, bytes: usize) -> bool;
}

/// A "heap-reclaimable" allocated stack - used for reclaiming the initial kernel stack once the task exits
/// Represents statically allocated memory (that's part of the heap/NX section in vmem) that is used as a stack,
///  but may be re-used as extra heap memory once it's done with (as it does not belong to an allocator already)
#[derive(Debug)]
pub struct HeapReclaimableAllocatedStack {
    start: usize,
    end: usize,
}
impl HeapReclaimableAllocatedStack {
    pub unsafe fn new(start: *mut u8, end: *mut u8) -> Self {
        Self { start: start as usize, end: end as usize }
    }
}
impl AnyAllocatedStack for HeapReclaimableAllocatedStack {
    fn bottom_vaddr(&self) -> usize {
        self.end as usize
    }

    fn expand(&mut self, bytes: usize) -> bool {
        false  // not supported
    }
}
impl Drop for HeapReclaimableAllocatedStack {
    fn drop(&mut self) {
        unsafe { super::kernel_heap::reclaim_for_heap(self.start as *mut u8, self.end as *mut u8); }
    }
}

pub struct DynOffsetMappedStack {
    allocation: OffsetMappedAllocation,
    guard_allocation: Option<UnifiedVirtGuard>,
}
impl DynOffsetMappedStack {
    /// Allocate an offset-mapped kernel stack
    /// (ideal for bootstrapping ATs)
    pub fn alloc_new_kernel(stack_size: PageAllocationSizeT, guard_size: PageAllocationSizeT) -> Option<Self> {
        let stack_allocation = OffsetMappedAllocation::alloc_new(stack_size, pageFlags!(t:WRITEABLE))?;
        let guard_allocation = UnifiedAllocation::alloc_new(AllocationType::GuardPage(GuardPageType::StackLimit), guard_size);
        let guard_allocation_virt = KERNEL_PTABLE.allocate_at(PageAlignedAddressT::new(stack_allocation.get_virt_addr().get()-guard_size.get()), guard_size);
        let guard_allocation = match guard_allocation_virt {
            Some(gav) => Some(guard_allocation.map_vmem(Box::new(gav), pageFlags!(), PageAlignedOffsetT::new(0))),
            None => None,
        };
        Some(Self { allocation: stack_allocation, guard_allocation })
    }
}
impl Debug for DynOffsetMappedStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "DynOffsetMappedStack[TODO]")  // TODO
    }
}

pub const MARKER_NULL_GUARD: usize = GuardPageType::NullPointer as usize;
pub fn new_user_paging_context() -> PagingContext {
    klog!(Debug, MEMORY_ALLOCUTIL, "Creating new user paging context.");
    let context = PagingContext::new();

    // null guard - 1MiB at the start to catch any null pointers
    const NULL_GUARD_SIZE: PageAllocationSizeT = PageAllocationSizeT::new_const(1*1024*1024);
    let nullguard = context.allocate_at(PageAlignedAddressT::new(0), NULL_GUARD_SIZE).unwrap();
    nullguard.set_absent(MARKER_NULL_GUARD);
    nullguard.leak();  // (we'll never need to de-allocate the null guard)

    // :)
    context
}