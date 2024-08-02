
// stack
use crate::memory::physical::{PhysicalMemoryAllocation,palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK};
use crate::memory::paging::global_pages::{GlobalPageAllocation,KERNEL_PTABLE};
fn allocate_ktask_stack() -> (PhysicalMemoryAllocation,GlobalPageAllocation){
    todo!()
}