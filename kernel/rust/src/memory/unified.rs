
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,MIN_PAGE_SIZE};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::hspin::{HMutex,HMutexGuard};

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
    pub fn end_addr(&self) -> usize {
        self.allocation.get_addr() + self.allocation.get_size()
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
    
    pub fn start_addr(&self) -> usize {
        self.allocation.start()
    }
    pub fn size(&self) -> usize {
        self.allocation.size()
    }
    pub fn end_addr(&self) -> usize {
        self.allocation.end()
    }
}

bitflags! {
    #[derive(Clone,Copy,Debug)]
    pub struct AllocationFlags : u32 {
        /// This may not be un-mapped from physical memory, or moved around within physical memory
        const STICKY = 1<<0;
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
    allocation_flags: AllocationFlags,
}
pub struct CombinedAllocation(HMutex<CombinedAllocationInner>);
impl CombinedAllocation {  // TODO: Figure out visibility n stuff
    pub fn new_from_phys(alloc: PhysicalMemoryAllocation, phys_flags: PMemFlags, alloc_flags: AllocationFlags) -> Arc<Self> {
        Arc::new(Self(HMutex::new(CombinedAllocationInner{
            sections: VecDeque::from([CombinedAllocationSegment{
                    size: alloc.get_size(),
                    physical: Some(PhysicalAllocation::new(
                        alloc, phys_flags,
                    )),
                    vmem: Vec::new(),
                    swap: None,
                }]),
            available_virt_slots: Vec::new(),
            
            physical_alloc_flags: phys_flags,
            allocation_flags: alloc_flags,
        })))
    }
    pub fn alloc_new_phys(size: usize, phys_flags: PMemFlags, alloc_flags: AllocationFlags) -> Option<Arc<Self>> {
        let layout = core::alloc::Layout::from_size_align(size, MIN_PAGE_SIZE).unwrap().pad_to_align();
        let phys_allocation = palloc(layout)?;
        Some(Self::new_from_phys(phys_allocation,phys_flags,alloc_flags))
    }
    pub fn alloc_new_phys_virt<PFA:PageFrameAllocator+Send+Sync+'static>(size: usize, phys_flags: PMemFlags, allocator: &LockedPageAllocator<PFA>, strategy: PageAllocationStrategies, virt_flags: VMemFlags, alloc_flags: AllocationFlags) -> Option<VirtAllocationGuard> {
        let allocation = Self::alloc_new_phys(size, phys_flags, alloc_flags)?;
        allocation.map_virtual(allocator, strategy, virt_flags)
    }
    
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
                assert!(expansion.size() <= new_section.size);
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
    
    fn map_virtual_inner<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, valloc:impl FnOnce(Option<usize>,usize)->Option<PageAllocation<PFA>>, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        let mut inner = self.0.lock(); let n_sections = inner.sections.len();
        // Sum size and determine split positions
        let mut total_size = 0;
        let mut split_lengths = Vec::<usize>::with_capacity(n_sections);  // final one isn't used but eh
        for section in inner.sections.iter() {
            total_size += section.size;
            split_lengths.push(section.size);
        }
        let _=split_lengths.pop();  // Pop the final one as it isn't needed
        
        let phys_addr = inner.sections[0].physical.as_ref().map(|p|p.start_addr());  // (used by valloc)
        // Allocate one big block of [SIZE], then split it into each section
        let mut lhs: PageAllocation<PFA>;
        let mut remainder = valloc(phys_addr,total_size)?; remainder.normalize();
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
    /// Map this into virtual memory in the given allocator
    pub fn map_virtual<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, allocator: &LockedPageAllocator<PFA>, strategy: PageAllocationStrategies, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        self.map_virtual_inner(|_phys_start,total_size|allocator.allocate(total_size,strategy), flags)
    }
    /// Map this into virtual memory, in the given allocator, at the given offset
    pub fn map_virtual_offset<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, allocator: &LockedPageAllocator<PFA>, offset: usize, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        self.map_virtual_inner(|phys_start,total_size|allocator.allocate_at(phys_start?+offset, total_size), flags)
    }
}

/// A guard corresponding to a given virtual memory allocation + its unified counterpart
pub struct VirtAllocationGuard {
    allocation: Arc<CombinedAllocation>,
    index: usize,
}
impl VirtAllocationGuard {
    pub fn combined_allocation(&self) -> &Arc<CombinedAllocation> {
        &self.allocation
    }
    fn combined_allocation_inner(&self) -> HMutexGuard<'_,CombinedAllocationInner> {
        core::ops::Deref::deref(&self.allocation).0.lock()
    }
    
    pub fn start_addr(&self) -> usize {
        let inner = self.combined_allocation_inner();
        inner.sections.iter().map(|s|&s.vmem[self.index]).flatten().next().unwrap().start_addr()
    }
    pub fn end_addr(&self) -> usize {
        let inner = self.combined_allocation_inner();
        inner.sections.iter().map(|s|&s.vmem[self.index]).flatten().next_back().unwrap().end_addr()
    }
}
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

impl super::alloc_util::AnyAllocatedStack for VirtAllocationGuard {
    // assumes the stack grows downwards
    fn bottom_vaddr(&self) -> usize {
        self.start_addr()
    }
    fn expand(&mut self, bytes: usize) -> bool {
        let start = self.start_addr();
        self.combined_allocation().expand_downwards(bytes);  // on success, this will change our start_addr
        start != self.start_addr()
    }
}
impl core::fmt::Debug for VirtAllocationGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VirtAllocationGuard [ {:x}..{:x} ]", self.start_addr(), self.end_addr())
    } 
}