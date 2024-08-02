use crate::logging::klog;
use core::alloc::Layout;

// stack
use crate::memory::physical::{PhysicalMemoryAllocation,palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK,PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
use crate::memory::paging::global_pages::{GlobalPageAllocation,KERNEL_PTABLE};

const KTASK_STACK_SIZE: usize = 256*1024;  // 256KiB TODO: figure this out better idk
const KSTACK_GUARD_SIZE: usize = 1;
pub fn allocate_ktask_stack() -> Option<(PhysicalMemoryAllocation,(GlobalPageAllocation,GlobalPageAllocation),usize)> {  // phys, (guard, stack), bottom_addr
    klog!(Debug, SCHEDULER_KTASK, "Allocating new kernel task stack.");
    let physmemallocation = palloc(Layout::from_size_align(KTASK_STACK_SIZE, 1).unwrap())?;
    let vmemallocation = KERNEL_PTABLE.allocate(physmemallocation.get_size() + KSTACK_GUARD_SIZE, KALLOCATION_KERNEL_STACK)?;
    let (guard, stack) = vmemallocation.split(KSTACK_GUARD_SIZE);  // split at the low-end as stack grows downwards
    
    guard.set_absent(0xF47B33F);
    stack.set_base_addr(physmemallocation.get_addr(), PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()));
    let bottom_addr = stack.end();
    
    klog!(Debug, SCHEDULER_KTASK, "Allocated kernel task stack at guard={:x} top={:x} bottom={:x} -> addr={:x},size={:x}.", guard.start(), stack.start(), bottom_addr, physmemallocation.get_addr(), physmemallocation.get_size());
    Some((physmemallocation, (guard, stack), bottom_addr))
}