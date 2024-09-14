
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::sync::{Arc,Weak};
use super::paging::AnyPageAllocation;
use super::physical::PhysicalMemoryAllocation;
use bitflags::bitflags;
use crate::sync::hspin::{HRwLock,HRwLockWriteGuard};

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

pub struct CombinedAllocationInner {  // vmem is placed before pmem as part of drop order - physical memory must be freed last
    vmem: Vec<Option<VirtualAllocation>>,  // indicies must always point to the same allocation, so we instead tombstone empty slots
    physical: Option<PhysicalAllocation>,
    swap: Option<()>,  // swap isn't supported yet
    
    /// Size in bytes
    size: usize,
    // todo: general flags
}
pub struct CombinedAllocation(HRwLock<CombinedAllocationInner>);
impl CombinedAllocation {  // TODO: Figure out visibility n stuff
    pub fn new(backing: PhysicalMemoryAllocation) -> Arc<Self> {
        Arc::new(Self(HRwLock::new(CombinedAllocationInner{
            size: backing.get_size(),
            
            physical: Some(PhysicalAllocation::new(backing, PMemFlags::empty())),
            vmem: Vec::new(),
            swap: None,
        })))
    }
    
    fn _map_to_current(self: &Arc<Self>, inner: &impl core::ops::Deref<Target=CombinedAllocationInner>, valloc: &VirtualAllocation){
        match inner.physical {
            Some(ref phys) => {
                valloc.map(phys.start_addr());  // TODO: handle baseaddr_offset??
            },
            None => {
                valloc.set_absent(0);  // TODO: data
            },
        }
    }
    /// Set the current physical allocation, dropping the old one
    fn set_physical(self: &Arc<Self>, new: Option<PhysicalAllocation>) {
        let mut inner = self.0.write();
        // Clear the old allocation
        let old = core::mem::replace(&mut inner.physical, new);
        drop(old);
        // (downgrade the guard)
        let inner = HRwLockWriteGuard::downgrade(inner);
        // Re-map all pages to point to it
        inner.vmem.iter().flatten().for_each(|valloc|self._map_to_current(&inner,valloc));
    }
    /// Add a new virtual allocation, returning a corresponding guard
    pub fn add_virtual_mapping(self: &Arc<Self>, virt: impl AnyPageAllocation + 'static, page_flags: VMemFlags) -> VirtAllocationGuard {
        // Create allocation object
        let virt = VirtualAllocation::new(virt, page_flags);
        assert!(virt.allocation.size() == self.size());  // size must be exactly equal (TODO: find a better way to check this)
        
        // Add to the list
        let mut inner = self.0.write();
        // Try overwriting a tombstoned one
        let index = inner.vmem.iter().position(|i|i.is_none()).ok_or(inner.vmem.len());
        // Ok(x) = overwrite, Err(x) = push
        let index = match index {
            Ok(index) => { inner.vmem[index] = Some(virt); index },
            Err(new_index) => { inner.vmem.push(Some(virt)); new_index },
        };
        
        // Map to current physical
        let inner = HRwLockWriteGuard::downgrade(inner);
        self._map_to_current(&inner, inner.vmem[index].as_ref().unwrap());
        
        // Return a guard
        VirtAllocationGuard {
            allocation: Arc::clone(self),
            index,
        }
    }
    
    pub fn size(self: &Arc<Self>) -> usize {
        self.0.read().size
    }
}

/// A guard corresponding to a given virtual memory allocation + its unified counterpart
pub struct VirtAllocationGuard {
    allocation: Arc<CombinedAllocation>,
    index: usize,
}
// TODO
impl Drop for VirtAllocationGuard {
    fn drop(&mut self) {
        // Clear our virtual allocation
        let mut guard = self.allocation.0.write();
        let vmem_allocation = guard.vmem[self.index].take();
        // Drop the guard first (now that we've tombstoned our slot in the allocation)
        drop(guard);
        // Then, drop the allocation (doing it in this order prevents deadlocks, as we only hold one lock at a time)
        debug_assert!(vmem_allocation.is_some());
        drop(vmem_allocation);
    }
}