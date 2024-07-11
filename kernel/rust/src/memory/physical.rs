use core::alloc::{Layout,GlobalAlloc};

use buddy_system_allocator::LockedHeap;

use crate::lowlevel::without_interrupts;

// Allocations must be page-aligned
const MIN_ALIGNMENT: usize = 4096;

static PHYSMEM_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();

pub struct PhysicalMemoryAllocation {
    ptr: core::ptr::NonNull<u8>,
    layout: Layout,
    size: usize,
}
impl PhysicalMemoryAllocation {
    pub fn get_ptr(&self) -> core::ptr::NonNull<u8> { self.ptr }
    pub fn get_size(&self) -> usize { self.size }
}
// Memory allocations cannot be copied nor cloned,
// as that would allow for use-after-free or double-free errors
// Physical memory allocations are managed by Rust's ownership rules
// (though it is still the responsibility of the kernel to ensure userspace processes
//      cannot access physical memory that has been freed or re-used)
impl !Copy for PhysicalMemoryAllocation{}
impl !Clone for PhysicalMemoryAllocation{}
impl !Sync for PhysicalMemoryAllocation{}

pub fn palloc(layout: Layout) -> Option<PhysicalMemoryAllocation> {
    let (ptr,size) = without_interrupts(||{
        let allocator = PHYSMEM_ALLOCATOR.lock();
        let old_actual = allocator.stats_alloc_actual();
        let ptr = PHYSMEM_ALLOCATOR.lock().alloc(layout).ok()?;
        let size = allocator.stats_alloc_actual()-old_actual;
        Some((ptr,size))
    })?;
    Some(PhysicalMemoryAllocation {
        ptr: ptr,
        layout: layout,
        size: size,
    })
}
pub fn pfree(allocation: PhysicalMemoryAllocation){
    without_interrupts(||{
        PHYSMEM_ALLOCATOR.lock().dealloc(allocation.ptr, allocation.layout)
    })
}