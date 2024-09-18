
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,MIN_PAGE_SIZE};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::hspin::{HMutex,HMutexGuard};

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

macro_rules! vec_of_non_clone {
    [$item:expr ; $count:expr] => {
        Vec::from_iter((0..$count).map(|_|$item))
    }
}

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
    
    /// Used if the page is not in physmem
    absent_pages_table_handle: AbsentPagesHandleA,  // (dropped after the vmem allocation has been erased)
}
impl VirtualAllocation {
    pub fn new(alloc: Box<dyn AnyPageAllocation>, flags: VMemFlags,
                  combined_alloc: Weak<CombinedAllocation>, virt_index: usize) -> (Self,AbsentPagesHandleB) {
        // Populate an absent_pages_table entry
        let ate = ABSENT_PAGES_TABLE.create_new_descriptor();
        let apth = ate.commit(AbsentPagesItemA {
            allocation: combined_alloc,
            virt_allocation_index: virt_index,
        }, AbsentPagesItemB {});
        let apth_a = apth.clone_a_ref();
        
        // Return self
        (Self {
            allocation: alloc,
            flags: flags,
            absent_pages_table_handle: apth_a,
        }, apth)
    }
    /// Map to a given physical address
    pub fn map(&self, phys_addr: usize) {
        self.allocation.set_base_addr(phys_addr, self.flags);
    }
    /// Set as absent
    pub fn set_absent(&self) {
        self.allocation.set_absent(self.absent_pages_table_handle.get_id().try_into().unwrap());
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
#[derive(Debug,Clone,Copy)]
pub enum VirtualAllocationMode {
    Dynamic { strategy: PageAllocationStrategies<'static> },
    OffsetMapped { offset: usize },
    FixedVirtAddr { addr: usize },
}
/// Lookup an "absent page" data item in the ABSENT_PAGES_TABLE
pub fn lookup_absent_id(absent_id: usize) -> Option<(Arc<CombinedAllocation>,usize)> {
    let apth_a = ABSENT_PAGES_TABLE.acquire_a(absent_id.try_into().unwrap()).ok()?;
    let apt_item_a = apth_a.get_a();
    let combined_alloc = Weak::upgrade(&apt_item_a.allocation)?;
    let virt_index = apt_item_a.virt_allocation_index;
    Some((combined_alloc,virt_index))
}

#[derive(Clone,Copy,Debug)]
pub enum GuardPageType {
    StackLimit = 0xF47B33F,  // Fat Beef
    NullPointer = 0x4E55_4C505452,  // "NULPTR"
}
pub enum SwapAllocation {
    /// Guard page - i.e. the memory doesn't actually exist, but is instead reserved to prevent null pointer derefs, buffer overruns, etc.
    GuardPage(GuardPageType),
    /// Uninitialised memory - memory that has been reserved in vmem, but not in pmem
    Uninitialised,
    
    // "real" swap isn't supported yet
}

bitflags! {
    #[derive(Clone,Copy,Debug)]
    pub struct AllocationFlags : u32 {
        /// This may not be un-mapped from physical memory, or moved around within physical memory
        const STICKY = 1<<0;
    }
}
// NOTE: CombinedAllocation must ALWAYS be locked BEFORE any page allocators (if you are nesting the locks, which isn't recommended but often necessary)!!
struct CombinedAllocationSegment {  // vmem is placed before pmem as part of drop order - physical memory must be freed last
    vmem: Vec<Option<VirtualAllocation>>,  // indicies must always point to the same allocation, so we instead tombstone empty slots
    physical: Option<PhysicalAllocation>,
    swap: Option<SwapAllocation>,
    
    /// Size in bytes
    size: usize,
    /// Section ID - a unique way of identifying a given section. Not equivalent to the index, which may vary as allocations and deallocations occur.
    section_identifier: usize,
}
impl CombinedAllocationSegment {
    pub fn _map_to_current(&self, valloc: &VirtualAllocation){
        match self.physical {
            Some(ref phys) => {
                valloc.map(phys.start_addr());  // valloc is normalized upon being added to the combinedallocation so we don't need to account for baseaddr_offset
            },
            None => {
                valloc.set_absent();
            },
        }
    }
    pub fn _update_mappings_section(&self){
        for valloc in self.vmem.iter().flatten() {
            self._map_to_current(valloc);
        }
    }
}
struct CombinedAllocationInner {
    sections: VecDeque<CombinedAllocationSegment>,  // a series of consecutive allocations in virtual memory, sorted in order of start address (from lowest to highest)
    available_virt_slots: Vec<Option<VirtualAllocationMode>>, // https://godbolt.org/z/3vE4nzzaM - Option<enum X> is optimized to a single tag instead of two (provided there's space)
    
    next_section_identifier: usize,
    physical_alloc_flags: PMemFlags,
    allocation_flags: AllocationFlags,
}
impl CombinedAllocationInner {
    pub fn _get_new_section_identifier(&mut self) -> usize {
        let id = self.next_section_identifier;
        self.next_section_identifier += 1;
        id
    }
    pub fn _find_section_with_identifier_index(&mut self, section_identifier: usize) -> Option<usize> {
        self.sections.iter_mut().position(|item|item.section_identifier == section_identifier)
    }
    pub fn _find_section_with_identifier_mut(&mut self, section_identifier: usize) -> Option<&mut CombinedAllocationSegment> {
        self.sections.iter_mut().find(|item|item.section_identifier == section_identifier)
    }
    
    pub fn _update_mappings_valloc(&self, valloc_idx: usize){
        for section in self.sections.iter() {
            let Some(valloc) = section.vmem[valloc_idx].as_ref() else { continue };
            section._map_to_current(valloc);
        }
    }
}
pub struct CombinedAllocation(HMutex<CombinedAllocationInner>);
impl CombinedAllocation {
    fn _new_single(size: usize, phys: Option<PhysicalMemoryAllocation>, phys_flags: PMemFlags,
                    swap: Option<SwapAllocation>, alloc_flags: AllocationFlags) -> Arc<Self> {
        Arc::new(Self(HMutex::new(CombinedAllocationInner{
            sections: VecDeque::from([CombinedAllocationSegment{
                    size: size,
                    physical: phys.map(|alloc|PhysicalAllocation::new(
                        alloc, phys_flags,
                    )),
                    vmem: Vec::with_capacity(1),
                    swap: swap,
                    section_identifier: 0,
                }]),
            available_virt_slots: Vec::with_capacity(1),
            
            next_section_identifier: 1,
            physical_alloc_flags: phys_flags,
            allocation_flags: alloc_flags,
        })))
    }
    pub fn new_from_phys(alloc: PhysicalMemoryAllocation, phys_flags: PMemFlags, alloc_flags: AllocationFlags) -> Arc<Self> {
        Self::_new_single(alloc.get_size(), Some(alloc), phys_flags, None, alloc_flags)
    }
    pub fn new_from_swap(size: usize, alloc: SwapAllocation, phys_flags: PMemFlags, alloc_flags: AllocationFlags) -> Arc<Self> {
        Self::_new_single(size, None, phys_flags, Some(alloc), alloc_flags)
    }
    pub fn alloc_new_phys(size: usize, phys_flags: PMemFlags, alloc_flags: AllocationFlags) -> Option<Arc<Self>> {
        let layout = core::alloc::Layout::from_size_align(size, MIN_PAGE_SIZE).unwrap().pad_to_align();
        let phys_allocation = palloc(layout)?;
        Some(Self::new_from_phys(phys_allocation,phys_flags,alloc_flags))
    }
    pub fn alloc_new_phys_virt<PFA:PageFrameAllocator+Send+Sync+'static>(size: usize, phys_flags: PMemFlags, allocator: &LockedPageAllocator<PFA>, virt_mode: VirtualAllocationMode, virt_flags: VMemFlags, alloc_flags: AllocationFlags) -> Option<VirtAllocationGuard> {
        let allocation = Self::alloc_new_phys(size, phys_flags, alloc_flags)?;
        allocation.map_virtual(allocator, virt_mode, virt_flags)
    }
    pub fn alloc_new_guard_virt_at<PFA:PageFrameAllocator+Send+Sync+'static>(addr: usize, size: usize, guard_type: GuardPageType, allocator: &LockedPageAllocator<PFA>, alloc_flags: AllocationFlags) -> Option<VirtAllocationGuard> {
        let allocation = Self::new_from_swap(size, SwapAllocation::GuardPage(guard_type), PMemFlags::empty(), alloc_flags);
        allocation.map_virtual(allocator, VirtualAllocationMode::FixedVirtAddr { addr }, VMemFlags::empty())
    }
    
    /// Allocate more memory, expanding downwards
    pub fn expand_downwards(self: &Arc<Self>, size_bytes: usize) {
        let mut inner = self.0.lock();
        let layout = core::alloc::Layout::from_size_align(size_bytes, MIN_PAGE_SIZE).unwrap().pad_to_align();
        // Begin setting up new section
        let mut new_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            physical: None,
            swap: None,
            
            size: layout.size(),
            section_identifier: inner._get_new_section_identifier(),
        };
        // Populate virtual memory allocations
        for (index, virt_mode) in inner.available_virt_slots.iter().enumerate() {
            new_section.vmem.push(try{
                let virt_mode = virt_mode.as_ref()?;
                if let VirtualAllocationMode::OffsetMapped{..} = virt_mode { None? }  // We can't expand offset-mapped allocations at the moment (as the expanded block of physmem could be anywhere)
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
                let (va, _) = VirtualAllocation::new(expansion, virt_flags, Arc::downgrade(self), index);
                va
            });
        }
        
        // Allocate backing (physical memory)
        let phys_flags = inner.physical_alloc_flags;
        new_section.physical = palloc(layout).map(|phys_raw|PhysicalAllocation::new(phys_raw, phys_flags));
        
        // Update mappings
        new_section._update_mappings_section();
        // And append
        inner.sections.push_front(new_section);
    }
    /// Split a section into two new sections, with new identifiers
    /// The lower section is guaranteed to contain at least `mid` bytes, though this may be rounded based on page alignment
    /// Returns Err if the section does not exist. Otherwise, returns the two section IDs
    pub fn split_section(self: &Arc<Self>, section_identifier: usize, mid: usize) -> Result<(usize,usize),()> {
        let mut inner = self.0.lock();
        let section_idx = inner._find_section_with_identifier_index(section_identifier).ok_or(())?;
        
        let mut old_section = inner.sections.remove(section_idx).unwrap();
        let left_size = core::alloc::Layout::from_size_align(mid, MIN_PAGE_SIZE).unwrap().pad_to_align().size();
        let right_size = old_section.size-left_size;
        
        let mut left_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            physical: None,
            swap: None,
            
            size: left_size,
            section_identifier: inner._get_new_section_identifier(),
        };
        let mut right_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            physical: None,
            swap: None,
            
            size: right_size,
            section_identifier: inner._get_new_section_identifier(),
        };
        let left_identifier = left_section.section_identifier;
        let right_identifier = right_section.section_identifier;
        
        // TODO physical/swap adjustments
        
        // Split virtual memory allocations
        for (index, valloc) in old_section.vmem.drain(0..).enumerate().filter_map(|(i,o)|o.map(|x|(i,x))) {
            let VirtualAllocation { allocation, flags, .. } = valloc;
            let (left_allocation, right_allocation) = allocation.split_dyn(left_size);
            debug_assert!(left_allocation.size()==left_size);
            debug_assert!(right_allocation.size()==right_size);
            
            let (la,_) = VirtualAllocation::new(
                left_allocation, flags.clone(), Arc::downgrade(self), index
            );
            left_section.vmem[index] = Some(la);
            let (ra, _) = VirtualAllocation::new(
                right_allocation, flags.clone(), Arc::downgrade(self), index
            );
            right_section.vmem[index] = Some(ra);
        }
        
        // Update mappings
        left_section._update_mappings_section();
        right_section._update_mappings_section();
        // Insert into list : list now = [X-2] [X-1] LHS RHS [X+1] [X+2], where X±y was relative to the old section
        inner.sections.insert(section_idx, right_section);
        inner.sections.insert(section_idx, left_section);
        // Return
        Ok((left_identifier, right_identifier))
    }
    
    /// Load a section from swap into physical memory
    pub fn swap_in(self: &Arc<Self>, section_identifier: usize) -> Result<(),SwapInError> {
        let mut inner = self.0.lock();
        let phys_alloc_flags = inner.physical_alloc_flags;
        let section = inner._find_section_with_identifier_mut(section_identifier).ok_or(SwapInError::SectionNotFound)?;
        
        // Check swap type to ensure it's valid
        let swap_type = section.swap.as_ref().ok_or(SwapInError::NotInSwap)?;
        match *swap_type {
            SwapAllocation::GuardPage(gp_type) => return Err(SwapInError::GuardPage(gp_type)),
            _=>{},
        }
        
        // Allocate physical memory
        let layout = core::alloc::Layout::from_size_align(section.size, MIN_PAGE_SIZE).unwrap();
        let phys_allocation = palloc(layout).ok_or(SwapInError::PMemAllocationFail)?;
        let phys_allocation = PhysicalAllocation { allocation: phys_allocation, flags: phys_alloc_flags };
        
        // TODO: Map into virtual memory and write data (if necessary)
        
        // Change state
        section.physical = Some(phys_allocation);
        section.swap = None;
        // Update mappings
        section._update_mappings_section();
        // Done :)
        Ok(())
    }
    
    /// Map this into virtual memory in the given allocator
    pub fn map_virtual<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, allocator: &LockedPageAllocator<PFA>, virt_mode: VirtualAllocationMode, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        let mut inner = self.0.lock(); let n_sections = inner.sections.len();
        // Sum size and determine split positions
        let mut total_size = 0;
        let mut split_lengths = Vec::<usize>::with_capacity(n_sections);  // final one isn't used but eh
        for section in inner.sections.iter() {
            total_size += section.size;
            split_lengths.push(section.size);
        }
        let _=split_lengths.pop();  // Pop the final one as it isn't needed
        
        // Allocate one big block of [SIZE]
        let phys_addr = inner.sections[0].physical.as_ref().map(|p|p.start_addr());
        let virt_allocation = match virt_mode {
            VirtualAllocationMode::Dynamic { strategy } => allocator.allocate(total_size,strategy),
            VirtualAllocationMode::OffsetMapped { offset } => allocator.allocate_at(phys_addr?+offset, total_size),
            VirtualAllocationMode::FixedVirtAddr { addr } => allocator.allocate_at(addr, total_size),
        };
        // then split it into each section
        let mut lhs: PageAllocation<PFA>;
        let mut remainder = virt_allocation?; remainder.normalize();
        let mut section_allocs = Vec::<PageAllocation<PFA>>::new();
        for split_size in split_lengths {
            (lhs, remainder) = remainder.split(split_size);
            debug_assert!(lhs.size() == split_size);
            section_allocs.push(lhs);
        }
        section_allocs.push(remainder);  // The final section
        
        // Reserve an index (must be done before initialising absent_pages_table entries)
        // Try overwriting a tombstoned slot
        let index_result = inner.available_virt_slots.iter().position(|i|i.is_none()).ok_or(inner.available_virt_slots.len());
        let index = match index_result {
            Ok(index) => {inner.available_virt_slots[index] = Some(virt_mode);index},  // by setting it to Some() we reserve the slot
            Err(new_index) => {inner.available_virt_slots.push(Some(virt_mode));new_index},
        };
        
        // Convert from PageAllocation into VirtualAllocation
        let mut virtual_allocs = Vec::<VirtualAllocation>::with_capacity(n_sections);
        for allocation in section_allocs {
            let (va, _) = VirtualAllocation::new(
                Box::new(allocation) as Box<dyn AnyPageAllocation>,
                flags,
                Arc::downgrade(self), index,
            );
            virtual_allocs.push(va);
        }
        
        // Push items to list
        // Ok(x) = overwrite, Err(x) = push
        match index_result {
            Ok(_) => {
                for (section,virt) in inner.sections.iter_mut().zip(virtual_allocs) {
                    section.vmem[index] = Some(virt);
                }
                index
            },
            Err(new_index) => {
                for (section,virt) in inner.sections.iter_mut().zip(virtual_allocs) {
                    section.vmem.push(Some(virt));
                }
                new_index
            },
        };
        
        // Update mappings
        inner._update_mappings_valloc(index);
        // And return a guard
        Some(VirtAllocationGuard { allocation: Arc::clone(self), index: index })
    }
}
pub enum SwapInError {
    /// Requested section holds a guard page
    GuardPage(GuardPageType),
    /// Could not allocate physical memory
    PMemAllocationFail,
    /// Not swapped out
    NotInSwap,
    /// Section not found
    SectionNotFound,
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
        guard.available_virt_slots[self.index] = None;
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

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
struct AbsentPagesItemA {
    allocation: Weak<CombinedAllocation>,
    virt_allocation_index: usize,
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<(),AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();
}