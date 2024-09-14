
use alloc::boxed::Box;
use alloc::sync::Arc;
use super::paging::AnyPageAllocation;
use super::physical::PhysicalMemoryAllocation;
use bitflags::bitflags;

bitflags! {
    pub struct PMemFlags: u32 {
        /// Zero out the memory upon allocating it
        const INIT_ZEROED = 1<<0;
        /// Zero out the memory before deallocating it
        const DROP_ZEROED = 1<<1;
    }
}
struct PhysicalAllocation {
    allocation: PhysicalMemoryAllocation,
    flags: PMemFlags,
}
impl PhysicalAllocation {
    fn _init(&self) {
        if self.flags.contains(PMemFlags::INIT_ZEROED) {
            // Zero out the memory
            todo!()
        }
    }
    pub fn new(alloc: PhysicalMemoryAllocation, flags: PMemFlags) -> Self {
        let this = Self {
            allocation: alloc,
            flags: flags,
        };
        this._init();
        this
    }
}

use super::paging::PageFlags as VMemFlags;