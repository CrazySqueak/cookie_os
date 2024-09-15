
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,MIN_PAGE_SIZE};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::hspin::{HMutex};

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

bitflags! {
    #[derive(Clone,Copy)]
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
        self.allocation.set_base_addr(phys_addr, self.flags);
    }
    /// Set as absent
    pub fn set_absent(&self, data: usize) {
        // TODO: Figure out what the 'data' argument should be
        self.allocation.set_absent(data);
    }
}
// NOTE: CombinedAllocation must ALWAYS be locked BEFORE any page allocators (if you are nesting the locks, which isn't recommended but often necessary)!!
pub struct CombinedAllocationSegment {  // vmem is placed before pmem as part of drop order - physical memory must be freed last
    vmem: Vec<Option<VirtualAllocation>>,  // indicies must always point to the same allocation, so we instead tombstone empty slots
    physical: Option<PhysicalAllocation>,
    swap: Option<()>,  // swap isn't supported yet
    
    /// Size in bytes
    size: usize,
}
pub struct CombinedAllocationInner {
    sections: VecDeque<CombinedAllocationSegment>,  // a series of consecutive allocations in virtual memory, sorted in order of start address (from lowest to highest)
    available_virt_slots: Vec<bool>,
    
    physical_alloc_flags: PMemFlags,
    // todo: general flags
}
pub struct CombinedAllocation(HMutex<CombinedAllocationInner>);
impl CombinedAllocation {  // TODO: Figure out visibility n stuff
    fn _map_to_current(self: &Arc<Self>, section: &CombinedAllocationSegment, valloc: &VirtualAllocation){
        match section.physical {
            Some(ref phys) => {
                valloc.map(phys.start_addr());  // valloc is normalized upon being added to the combinedallocation so we don't need to account for baseaddr_offset
            },
            None => {
                valloc.set_absent(0);  // TODO: data
            },
        }
    }
    fn _update_mappings_section(self: &Arc<Self>, section: &CombinedAllocationSegment){
        for valloc in section.vmem.iter().flatten() {
            self._map_to_current(section, valloc);
        }
    }
    fn _update_mappings_valloc(self: &Arc<Self>, sections: &VecDeque<CombinedAllocationSegment>, valloc_idx: usize){
        for section in sections.iter() {
            let Some(valloc) = section.vmem[valloc_idx].as_ref() else { continue };
            self._map_to_current(section, valloc);
        }
    }
    
    /// Allocate more memory, expanding downwards
    pub fn expand_downwards(self: &Arc<Self>, size_bytes: usize) {
        let mut inner = self.0.lock();
        let layout = core::alloc::Layout::from_size_align(size_bytes, MIN_PAGE_SIZE).unwrap().pad_to_align();
        // Begin setting up new section
        let mut new_section = CombinedAllocationSegment {
            vmem: Vec::with_capacity(inner.available_virt_slots.len()),
            physical: None,
            swap: None,
            
            size: layout.size(),
        };
        // Populate virtual memory allocations
        for (index, active) in inner.available_virt_slots.iter().enumerate() {
            new_section.vmem.push(try{
                if !active { None? }
                let previous_bottom = inner.sections[0].vmem[index].as_ref()?;
                // Expand vmem allocation
                let mut expansion = previous_bottom.allocation.alloc_downwards_dyn(new_section.size)?;
                expansion.normalize();
                // Adjust size if necessary
                //if new_section.size == 0 { new_section.size = expansion.size(); }
                assert!(new_section.size == expansion.size());
                // Carry across flags from the lowest vmem allocation in the section
                let virt_flags = previous_bottom.flags.clone();
                // Return new allocation
                VirtualAllocation { allocation: expansion, flags: virt_flags }
            });
        }
        
        // Allocate backing (physical memory)
        let phys_flags = inner.physical_alloc_flags;
        new_section.physical = palloc(layout).map(|phys_raw|PhysicalAllocation::new(phys_raw, phys_flags));
        
        // Update mappings
        self._update_mappings_section(&new_section);
        // And append
        inner.sections.push_front(new_section);
    }
    
    /// Map this into virtual memory in the given allocator
    pub fn map_virtual<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, allocator: &LockedPageAllocator<PFA>, strategy: PageAllocationStrategies, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        let mut inner = self.0.lock(); let n_sections = inner.sections.len();
        // Sum size and determine split positions
        let mut total_size = 0;
        let mut split_lengths = Vec::<usize>::with_capacity(n_sections);  // final one isn't used but eh
        for section in inner.sections.iter() {
            total_size += section.size;
            split_lengths.push(section.size);
        }
        let _=split_lengths.pop();  // Pop the final one as it isn't needed
        
        // Allocate one big block of [SIZE], then split it into each section
        let mut lhs: PageAllocation<PFA>;
        let mut remainder = allocator.allocate(total_size, strategy)?; remainder.normalize();
        let mut section_allocs = Vec::<PageAllocation<PFA>>::new();
        for split_size in split_lengths {
            (lhs, remainder) = remainder.split(split_size);
            debug_assert!(lhs.size() == split_size);
            section_allocs.push(lhs);
        }
        section_allocs.push(remainder);  // The final section
        
        // Convert from PageAllocation into VirtualAllocation
        let mut virtual_allocs = Vec::<VirtualAllocation>::with_capacity(n_sections);
        for allocation in section_allocs {
            virtual_allocs.push(VirtualAllocation {
                allocation: Box::new(allocation) as Box<dyn AnyPageAllocation>,
                flags: flags,
            });
        }
        
        // Try overwriting a tombstoned slot
        let index = inner.available_virt_slots.iter().position(|i|*i).ok_or(inner.available_virt_slots.len());
        // Ok(x) = overwrite, Err(x) = push
        let index = match index {
            Ok(index) => {
                for (section,virt) in inner.sections.iter_mut().zip(virtual_allocs) {
                    section.vmem[index] = Some(virt);
                }
                inner.available_virt_slots[index] = false;
                index
            },
            Err(new_index) => {
                for (section,virt) in inner.sections.iter_mut().zip(virtual_allocs) {
                    section.vmem.push(Some(virt));
                }
                inner.available_virt_slots.push(false);
                new_index
            },
        };
        
        // Update mappings
        self._update_mappings_valloc(&inner.sections, index);
        // And return a guard
        Some(VirtAllocationGuard { allocation: Arc::clone(self), index: index })
    }
    // pub fn new(backing: PhysicalMemoryAllocation) -> Arc<Self> {
    //     Arc::new(Self(HRwLock::new(CombinedAllocationInner{
    //         size: backing.get_size(),
    //         
    //         physical: Some(PhysicalAllocation::new(backing, PMemFlags::empty())),
    //         vmem: Vec::new(),
    //         swap: None,
    //     })))
    // }
    
    // /// Set the current physical allocation, dropping the old one
    // fn set_physical(self: &Arc<Self>, new: Option<PhysicalAllocation>) {
    //     let mut inner = self.0.lock();
    //     // Clear the old allocation
    //     let old = core::mem::replace(&mut inner.physical, new);
    //     drop(old);
    //     // Re-map all pages to point to it
    //     inner.vmem.iter().flatten().for_each(|valloc|self._map_to_current(&inner,valloc));
    // }
    // /// Add a new virtual allocation, returning a corresponding guard
    // pub fn add_virtual_mapping(self: &Arc<Self>, virt: impl AnyPageAllocation + 'static, page_flags: VMemFlags) -> VirtAllocationGuard {
    //     // Create allocation object
    //     let virt = VirtualAllocation::new(virt, page_flags);
    //     assert!(virt.allocation.size() == self.size());  // size must be exactly equal (TODO: find a better way to check this)
    //     
    //     // Add to the list
    //     let mut inner = self.0.lock();
    //     // Try overwriting a tombstoned one
    //     let index = inner.vmem.iter().position(|i|i.is_none()).ok_or(inner.vmem.len());
    //     // Ok(x) = overwrite, Err(x) = push
    //     let index = match index {
    //         Ok(index) => { inner.vmem[index] = Some(virt); index },
    //         Err(new_index) => { inner.vmem.push(Some(virt)); new_index },
    //     };
    //     
    //     // Map to current physical
    //     self._map_to_current(&inner, inner.vmem[index].as_ref().unwrap());
    //     
    //     // Return a guard
    //     VirtAllocationGuard {
    //         allocation: Arc::clone(self),
    //         index,
    //     }
    // }
    // 
    // pub fn size(self: &Arc<Self>) -> usize {
    //     self.0.read().size
    // }
}

/// A guard corresponding to a given virtual memory allocation + its unified counterpart
pub struct VirtAllocationGuard {
    allocation: Arc<CombinedAllocation>,
    index: usize,
}
// TODO
impl Drop for VirtAllocationGuard {
    fn drop(&mut self) {
        // Clear our virtual allocations and free the slot
        let mut guard = self.allocation.0.lock();
        let mut vmem_allocations = Vec::<Option<VirtualAllocation>>::with_capacity(guard.sections.len()); 
        for section in guard.sections.iter_mut() { vmem_allocations.push(section.vmem[self.index].take()); }
        guard.available_virt_slots[self.index] = true;
        // Drop the guard first (now that we've tombstoned our slot in the allocation)
        drop(guard);
        // Then, drop the allocation (doing it in this order prevents deadlocks, as we only hold one lock at a time)
        drop(vmem_allocations);
    }
}
impl !Clone for VirtAllocationGuard{}