
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::sync::{Arc,Weak};
use super::paging::AnyPageAllocation;
use super::physical::PhysicalMemoryAllocation;
use bitflags::bitflags;
use crate::sync::hspin::{HRwLock};

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN};

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
    fn _zero_out(&self) {
        let phys_addr = self.allocation.get_addr();
        let size = self.allocation.get_size();
        // First, we map it somewhere
        let vmap = KERNEL_PTABLE.allocate(size, KALLOCATION_KERNEL_GENERALDYN).expect("How the fuck are you out of virtual memory???");
        let ptr = vmap.start() as *mut u8;
        // Then, we zero it out
        // SAFETY: We just allocated the vmem so we know it's valid
        unsafe { core::ptr::write_bytes(ptr, 0, size) }
        // All done
        drop(vmap);
    }
    
    fn _init(&self) {
        if self.flags.contains(PMemFlags::INIT_ZEROED) {
            // Clear memory ready for use
            self._zero_out();
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
    pub fn start_addr(&self) -> usize {
        self.allocation.get_addr()
    }
    pub fn size(&self) -> usize {
        self.allocation.get_size()
    }
}
impl core::ops::Drop for PhysicalAllocation {
    fn drop(&mut self) {
        if self.flags.contains(PMemFlags::DROP_ZEROED) {
            // Clear memory now that we're done with it
            self._zero_out();
        }
    }
}

use super::paging::PageFlags as VMemFlags;
struct VirtualAllocation {
    allocation: Box<dyn AnyPageAllocation>,
    flags: VMemFlags,
}
impl VirtualAllocation {
    pub fn new(alloc: impl AnyPageAllocation + 'static, flags: VMemFlags) -> Self {
        Self {
            allocation: Box::new(alloc) as Box<dyn AnyPageAllocation>,
            flags: flags,
        }
    }
    /// Map to a given physical address
    pub fn map(&self, phys_addr: usize) {
        // TODO: Handle baseaddr_offset???
        self.allocation.set_base_addr(phys_addr, self.flags);
    }
    /// Set as absent
    pub fn set_absent(&self, data: usize) {
        // TODO: Figure out what the 'data' argument should be
        self.allocation.set_absent(data);
    }
}

pub struct CombinedAllocation {  // vmem is placed before pmem as part of drop order - physical memory must be freed last
    vmem: Vec<Option<VirtualAllocation>>,  // indicies must always point to the same allocation, so we instead tombstone empty slots
    physical: Option<PhysicalAllocation>,
    swap: Option<()>,  // swap isn't supported yet
    // todo: general flags
    
    self_arc: CombinedAllocationWeak,
}
impl CombinedAllocation {  // TODO: Figure out visibility n stuff
    /// Set the current physical allocation, dropping the old one
    fn set_physical(&mut self, new: Option<PhysicalAllocation>) {
        // Clear the old allocation
        let old = core::mem::replace(&mut self.physical, new);
        drop(old);
        // Re-map all pages to point to it
        let vmem = self.vmem.iter().flatten();
        match self.physical {
            Some(ref new) => {
                for valloc in vmem {
                    valloc.map(new.start_addr())
                }
            },
            None => {
                for valloc in vmem {
                    valloc.set_absent(0);  // TODO: data
                }
            },
        }
    }
    /// Add a new virtual allocation, returning a corresponding guard
    fn add_virtual(&mut self, virt: VirtualAllocation) -> VirtAllocationGuard {
        // Try overwriting a tombstoned one
        let index = self.vmem.iter().position(|i|i.is_none()).ok_or(self.vmem.len());
        // Ok(x) = overwrite, Err(x) = push
        let index = match index {
            Ok(index) => { self.vmem[index] = Some(virt); index },
            Err(new_index) => { self.vmem.push(Some(virt)); new_index },
        };
        // Return a guard
        VirtAllocationGuard {
            allocation: CombinedAllocationWeak::upgrade(&self.self_arc).unwrap(),
            index,
        }
    }
}
type CombinedAllocationRc = Arc<HRwLock<CombinedAllocation>>;
type CombinedAllocationWeak = Weak<HRwLock<CombinedAllocation>>;

/// A guard corresponding to a given virtual memory allocation + its unified counterpart
pub struct VirtAllocationGuard {
    allocation: CombinedAllocationRc,
    index: usize,
}
// TODO
impl Drop for VirtAllocationGuard {
    fn drop(&mut self) {
        // Clear our virtual allocation
        let mut guard = self.allocation.write();
        let vmem_allocation = guard.vmem[self.index].take();
        // Drop the guard first (now that we've tombstoned our slot in the allocation)
        drop(guard);
        // Then, drop the allocation (doing it in this order prevents deadlocks, as we only hold one lock at a time)
        debug_assert!(vmem_allocation.is_some());
        drop(vmem_allocation);
    }
}