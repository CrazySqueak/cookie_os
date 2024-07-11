use core::alloc::{Layout,GlobalAlloc};
use alloc::vec::Vec;
use core::ptr::addr_of;

use buddy_system_allocator::LockedHeap;

use crate::lowlevel::without_interrupts;
use crate::lowlevel::multiboot::MemoryMapEntry;

use core::fmt::write;
use crate::util::LockedWrite;
use crate::coredrivers::serial_uart::SERIAL1;

// Memory occupied by the kernel (set by the linker script)
extern "C" {
    static kernel_phys_start: u8;
    static kernel_phys_end: u8;
}
/* Get the kernel's start and end in physical memory. (start inclusive, end exclusive)
This covers the executable, bss, etc. sections, but does not cover any memory that was dynamically allocated afterwards.
(N.B. since the initial kernel heap is defined in the .bss section, that IS included, but if the heap is expanded afterwards then the expanded parts won't be included.) */
pub fn get_kernel_bounds() -> (usize, usize) {
    unsafe {
        (addr_of!(kernel_phys_start) as usize, addr_of!(kernel_phys_end) as usize)
    }
}

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

pub fn init_pmem(mmap: &Vec<MemoryMapEntry>){
    let (_, kend) = get_kernel_bounds();  // note: we ignore any memory before the kernel, its a tiny sliver (2MB tops) and isn't worth it
    write!(SERIAL1, "\n\tKernel ends @ {:x}", kend);
    without_interrupts(||{
        let mut allocator = PHYSMEM_ALLOCATOR.lock();
        for entry in mmap {
            write!(SERIAL1, "\nChecking PMem entry {:?}", entry);
            unsafe {
                if !entry.is_for_general_use() { continue; }
                write!(SERIAL1, "\n\tEntry is for general use.");
                let start_addr: usize = entry.base_addr.try_into().unwrap();
                let end_addr: usize = (entry.base_addr + entry.length).try_into().unwrap();
                write!(SERIAL1, "\n\tRange: [{:x},{:x})", start_addr, end_addr);
                if start_addr >= kend {
                    // after the kernel
                    write!(SERIAL1, "\n\tAfter the kernel");
                    allocator.add_to_heap(start_addr, end_addr);
                } else if end_addr > kend {
                    // intersecting the kernel
                    write!(SERIAL1, "\n\tIntersects the kernel");
                    allocator.add_to_heap(kend, end_addr);
                }
            }
        }
    })
}

pub fn palloc(layout: Layout) -> Option<PhysicalMemoryAllocation> {
    let (ptr,size) = without_interrupts(||{
        let mut allocator = PHYSMEM_ALLOCATOR.lock();
        let old_actual = allocator.stats_alloc_actual();
        let ptr = allocator.alloc(layout).ok()?;
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