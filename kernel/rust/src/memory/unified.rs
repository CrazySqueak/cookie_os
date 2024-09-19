
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,MIN_PAGE_SIZE};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::{YMutex,YMutexGuard};

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

macro_rules! vec_of_non_clone {
    [$item:expr ; $count:expr] => {
        Vec::from_iter((0..$count).map(|_|$item))
    }
}

/*
pub enum PhysicalAllocationSharable {
    Owned(PhysicalMemoryAllocation),
    Shared { alloc: Arc<PhysicalMemoryAllocation>, offset: usize, size: usize },
}
impl PhysicalAllocationSharable {
    pub fn get_addr(&self) -> usize {
        match self {
            &Self::Owned(ref alloc) => alloc.get_addr(),
            &Self::Shared{ref alloc, offset,..} => alloc.get_addr()+offset,
        }
    }
    pub fn get_size(&self) -> usize {
        match self {
            &Self::Owned(ref alloc) => alloc.get_size(),
            &Self::Shared{size,..} => size,
        }
    }
    
    pub fn split(self, mid: usize) -> (Self,Self) {
        let (alloc, base_offset, base_size) = match self {
            Self::Owned(alloc) => {let size = alloc.get_size(); (Arc::new(alloc),0,size)},
            Self::Shared { alloc, offset, size} => (alloc,offset,size),
        };
        //let base_limit = base_offset+base_size;
        let mid = core::cmp::min(mid,base_size);
        
        let lhs = Self::Shared {
            alloc: Arc::clone(&alloc),
            offset: base_offset,
            size: mid,
        };
        let rhs = Self::Shared {
            alloc: alloc,
            offset: base_offset+mid,
            size: base_size-mid,
        };
        (lhs, rhs)
    }
}
*/

#[derive(Clone,Copy,Debug)]
pub enum GuardPageType {
    StackLimit = 0xF47B33F,  // Fat Beef
    NullPointer = 0x4E55_4C505452,  // "NULPTR"
}
pub type BackingSize = core::num::NonZero<usize>;
/// A request for allocation backing
/// WARNING: Requests are not guaranteed to contain a size that has been rounded up to the minimum page size
/// To get the size post-rounding, use the .get_size() method. Yes, even if you're using a match statement.
pub enum AllocationBackingRequest {
    UninitPhysical { size: BackingSize },
    ZeroedPhysical { size: BackingSize },
    /// Similar to UninitPhysical, but isn't automatically allocated in physical memory. Instead, it's initialised as an UninitMem.
    Reservation { size: BackingSize },
    
    GuardPage { gptype: GuardPageType, size: BackingSize },
}
impl AllocationBackingRequest {
    pub fn get_size_unrounded(&self) -> BackingSize {
        match *self {
            Self::UninitPhysical{size} => size,
            Self::ZeroedPhysical{size} => size,
            Self::Reservation{size} => size,
            Self::GuardPage{size,..} => size,
        }
    }
    /// Get the size, and round it to the next MIN_PAGE_SIZE
    pub fn get_size(&self) -> BackingSize {
        let rounded = core::alloc::Layout::from_size_align(self.get_size_unrounded().into(), MIN_PAGE_SIZE).unwrap().pad_to_align().size();
        BackingSize::new(rounded).unwrap()
    }
}
/// The memory that backs a given allocation
enum AllocationBackingMode {
    /// Backed by physical memory
    PhysMem(PhysicalMemoryAllocation),
    /// Backed by shared physical memory
    PhysMemShared { alloc: Arc<PhysicalMemoryAllocation>, offset: usize },
    /// "guard page" - attempting to access it causes a page fault
    GuardPage(GuardPageType),
    /// Uninitialised memory - when swapped in, memory will be left uninitialised
    UninitMem,
    /// Zeroed memory - when swapped in, memory will be zeroed
    Zeroed,
}
struct AllocationBacking {
    mode: AllocationBackingMode,
    size: BackingSize,
}
impl AllocationBacking {
    pub fn new(mode: AllocationBackingMode, size: BackingSize) -> Self {
        debug_assert!(size.get()%MIN_PAGE_SIZE==0);
        debug_assert!(size.get() > 0);
        Self { mode, size }
    }
    
    fn _palloc(size: usize) -> Option<PhysicalMemoryAllocation> {
        palloc(core::alloc::Layout::from_size_align(size, MIN_PAGE_SIZE).unwrap())
    }
    /// Returns (self,true) if the request was fulfilled immediately. Returns (self,false) if it couldn't, and must be swapped in later when more RAM is available
    pub fn new_from_request(request: AllocationBackingRequest) -> (Self,bool) {
        let size = request.get_size();
        match request {
            AllocationBackingRequest::GuardPage { gptype, .. } => (Self::new(AllocationBackingMode::GuardPage(gptype),size),true),
            AllocationBackingRequest::Reservation { size } => (Self::new(AllocationBackingMode::UninitMem,size),true),
            
            AllocationBackingRequest::UninitPhysical { .. } => {
                match Self::_palloc(size.get()) {
                    Some(pma) => (Self::new(AllocationBackingMode::PhysMem(pma),size),true),
                    None => (Self::new(AllocationBackingMode::UninitMem,size),false),
                }
            },
            AllocationBackingRequest::ZeroedPhysical { .. } => {
                let mut this = Self::new(AllocationBackingMode::Zeroed,size);
                match this.swap_in() {
                    Ok(_) => (this,true),
                    Err(_) => (this,false),
                }
            },
        }
    }
    
    /// Split this allocation into two. One of `midpoint` bytes and one of the remainder (if nonzero)
    pub fn split(self, midpoint: BackingSize) -> (Self,Option<Self>) {
        if midpoint >= self.size { return (self,None); }
        let midpoint: usize = midpoint.get();
        debug_assert!(midpoint%MIN_PAGE_SIZE==0);
        debug_assert!(midpoint != 0);
        let lhs_size = midpoint;
        let rhs_size = self.size.get()-midpoint;
        
        let (lhs_mode,rhs_mode) = match self.mode {
            AllocationBackingMode::PhysMem(allocation) => {
                let allocation = Arc::new(allocation);
                (AllocationBackingMode::PhysMemShared { alloc: Arc::clone(&allocation), offset: 0 },
                 AllocationBackingMode::PhysMemShared { alloc: allocation, offset: lhs_size })
            },
            AllocationBackingMode::PhysMemShared { alloc: allocation, offset } => {
                (AllocationBackingMode::PhysMemShared { alloc: Arc::clone(&allocation), offset: offset+0 },
                 AllocationBackingMode::PhysMemShared { alloc: allocation, offset: offset+lhs_size })
            },
            
            AllocationBackingMode::GuardPage(gptype) => (AllocationBackingMode::GuardPage(gptype),AllocationBackingMode::GuardPage(gptype)),
            AllocationBackingMode::UninitMem => (AllocationBackingMode::UninitMem,AllocationBackingMode::UninitMem),
            AllocationBackingMode::Zeroed => (AllocationBackingMode::Zeroed,AllocationBackingMode::Zeroed),
        };
        (Self::new(lhs_mode,BackingSize::new(lhs_size).unwrap()),
         Some(Self::new(rhs_mode,BackingSize::new(rhs_size).unwrap())))
    }
    
    /// Load into physical memory
    /// Returns Ok() if successful, Err() if it failed
    /// 
    /// Note: This may still be required even if get_addr() returns Some(). Only consider it to be "already swapped in so page fault is some other issue" if this returns AlreadyInPhysMem
    ///       For example, copy-on-write memory would be implemented as a CopyOnWrite backing type, and would mark the memory as read-only, requiring a swap-in to copy from the old backing memory to a fresh area
    pub fn swap_in(&mut self) -> Result<BackingLoadSuccess,BackingLoadError> {
        match self.mode {
            AllocationBackingMode::PhysMem(_) | AllocationBackingMode::PhysMemShared{..} => Ok(BackingLoadSuccess::AlreadyInPhysMem),
            AllocationBackingMode::GuardPage(gptype) => Err(BackingLoadError::GuardPage(gptype)),
            
            _ => {
                 let phys_alloc = Self::_palloc(self.size.get()).ok_or(BackingLoadError::PhysicalAllocationFailed)?;
                 match self.mode {
                     AllocationBackingMode::PhysMem(_) | AllocationBackingMode::PhysMemShared{..} | AllocationBackingMode::GuardPage(_) => unreachable!(),
                     
                     AllocationBackingMode::UninitMem => {},  // uninit mem can be left as-is
                     AllocationBackingMode::Zeroed => {  // zeroed and other backing modes must be mapped into vmem and initialised
                        // Map into vmem and obtain pointer
                        let vmap = KERNEL_PTABLE.allocate(self.size.get(), KALLOCATION_KERNEL_GENERALDYN).expect("How the fuck are you out of virtual memory???");
                        let ptr = vmap.start() as *mut u8;
                        // Initialise memory
                        match self.mode {
                            AllocationBackingMode::PhysMem(_) | AllocationBackingMode::PhysMemShared{..} | AllocationBackingMode::GuardPage(_) | AllocationBackingMode::UninitMem => unreachable!(),
                            
                            AllocationBackingMode::Zeroed => unsafe{
                                // SAFETY: This pointer is to vmem which we have just allocated, and is to u8s which are robust
                                // Guaranteed to be page-aligned and allocated
                                core::ptr::write_bytes(ptr, 0, self.size.get())
                            },
                        }
                        // Clear vmem allocation
                        drop(vmap)
                     },
                 }
                 self.mode = AllocationBackingMode::PhysMem(phys_alloc);
                 Ok(BackingLoadSuccess::LoadSuccessful)
            },
        }
    }
    
    /// Get the starting physical memory address, or None if not in physical memory right now
    pub fn get_addr(&self) -> Option<usize> {
        match self.mode {
            AllocationBackingMode::PhysMem(ref alloc) => Some(alloc.get_addr()),
            AllocationBackingMode::PhysMemShared{ ref alloc, offset } => Some(alloc.get_addr()+offset),
            _ => None,
        }
    }
    /// Get the size of this allocation
    pub fn get_size(&self) -> BackingSize {
        self.size
    }
}
pub enum BackingLoadSuccess {
    /// "already in phys mem" can be an error unto itself, in some cases
    AlreadyInPhysMem,
    LoadSuccessful,
}
pub enum BackingLoadError {
    GuardPage(GuardPageType),
    PhysicalAllocationFailed,
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
                  combined_alloc: Weak<CombinedAllocation>, virt_index: usize, section_identifier: usize) -> (Self,AbsentPagesHandleB) {
        // Populate an absent_pages_table entry
        let ate = ABSENT_PAGES_TABLE.create_new_descriptor();
        let apth = ate.commit(AbsentPagesItemA {
            allocation: combined_alloc,
            virt_allocation_index: virt_index,
            section_identifier: section_identifier,
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
pub fn lookup_absent_id(absent_id: usize) -> Option<(AllocationSection,usize)> {
    let apth_a = ABSENT_PAGES_TABLE.acquire_a(absent_id.try_into().unwrap()).ok()?;
    let apt_item_a = apth_a.get_a();
    let combined_alloc = Weak::upgrade(&apt_item_a.allocation)?;
    let virt_index = apt_item_a.virt_allocation_index;
    let section_identifier = apt_item_a.section_identifier;
    let section_obj = AllocationSection::new(combined_alloc,section_identifier);
    Some((section_obj,virt_index))
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
    /// Virtual memory mappings assigned to this allocation
    vmem: Vec<Option<VirtualAllocation>>,  // indicies must always point to the same allocation, so we instead tombstone empty slots
    /// The backing memory, whether physical, uninitialised, or swap
    backing: AllocationBacking,
    
    /// Size in bytes
    size: BackingSize,
    /// Section ID - a unique way of identifying a given section. Not equivalent to the index, which may vary as allocations and deallocations occur.
    section_identifier: usize,
}
impl CombinedAllocationSegment {
    pub fn _map_to_current(&self, valloc: &VirtualAllocation){
        debug_assert!(valloc.size() == self.backing.get_size().get());
        match self.backing.get_addr() {
            Some(addr) => {
                valloc.map(addr);  // valloc is normalized upon being added to the combinedallocation so we don't need to account for baseaddr_offset
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
pub struct CombinedAllocation(YMutex<CombinedAllocationInner>);
impl CombinedAllocation {
    fn _new_single(size: BackingSize, backing: AllocationBacking, alloc_flags: AllocationFlags) -> AllocationSection {
        let this = Arc::new(Self(YMutex::new(CombinedAllocationInner{
            sections: VecDeque::from([CombinedAllocationSegment{
                    size: size,
                    backing: backing,
                    vmem: Vec::with_capacity(1),
                    section_identifier: 0,
                }]),
            available_virt_slots: Vec::with_capacity(1),
            
            next_section_identifier: 1,
            allocation_flags: alloc_flags,
        })));
        AllocationSection::new(this, 0)
    }
    pub fn new_from_request(backing_req: AllocationBackingRequest, alloc_flags: AllocationFlags) -> (AllocationSection,bool) {
        let (backing,success) = AllocationBacking::new_from_request(backing_req);
        let size = backing.get_size();
        (Self::_new_single(size,backing,alloc_flags), success)
    }
    pub fn new_from_requests(backing_reqs: Vec<AllocationBackingRequest>, alloc_flags: AllocationFlags) -> Vec<AllocationSection> {
        let total_size = backing_reqs.iter().map(|r|r.get_size().get()).reduce(core::ops::Add::add).unwrap();  // total_size, rounded to account for page-alignment
        let (this,_) = Self::new_from_request(AllocationBackingRequest::Reservation { size: BackingSize::new(total_size).unwrap() }, alloc_flags);
        
        let mut finished: Vec<AllocationSection> = Vec::new();
        let mut size_remaining: usize = total_size;
        let mut remainder: AllocationSection = this;
        let final_request = 'lp:{for (num_left,request) in backing_reqs.into_iter().enumerate().rev() {
            if num_left == 0 { break 'lp request; } // Final request needs to be handled differently (as lhs size must not be zero)
            // Split from right-to-left
            let (lhs,rhs) = remainder.split(BackingSize::new(size_remaining).unwrap()).unwrap();
            // rhs = this current request
            let size = request.get_size();
            let ok = rhs.overwrite_type(request);
            assert!(ok);
            finished.push(rhs);
            // lhs = remainder
            remainder = lhs;
            size_remaining -= size.get();
        } panic!("End of loop but not broken out?");};
        let ok = remainder.overwrite_type(final_request);
        assert!(ok);
        finished.push(remainder);
        
        // All good
        finished.reverse();  // reverse order so that earlier allocations are earlier in the vec
        finished  // and return
    }
    
    /// Allocate more memory, expanding downwards
    pub fn expand_downwards(self: &Arc<Self>, request: AllocationBackingRequest) -> AllocationSection {
        let mut inner = self.0.lock();
        
        // Allocate backing
        let (backing, _) = AllocationBacking::new_from_request(request);
        let size = backing.get_size();
        
        // Begin setting up new section
        let mut new_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            
            backing: backing,
            size: size,
            
            section_identifier: inner._get_new_section_identifier(),
        };
        
        // Populate virtual memory allocations
        for (index, virt_mode) in inner.available_virt_slots.iter().enumerate() {
            new_section.vmem.push(try{
                let virt_mode = virt_mode.as_ref()?;
                if let VirtualAllocationMode::OffsetMapped{..} = virt_mode { None? }  // We can't expand offset-mapped allocations at the moment (as the expanded block of physmem could be anywhere)
                let previous_bottom = inner.sections[0].vmem[index].as_ref()?;
                // Expand vmem allocation
                let mut expansion = previous_bottom.allocation.alloc_downwards_dyn(new_section.size.get())?;
                expansion.normalize();
                // Adjust size if necessary
                //if new_section.size == 0 { new_section.size = expansion.size(); }
                assert!(expansion.size() <= new_section.size.get());
                // Carry across flags from the lowest vmem allocation in the section
                let virt_flags = previous_bottom.flags.clone();
                // Return new allocation
                let (va, _) = VirtualAllocation::new(expansion, virt_flags, Arc::downgrade(self), index, new_section.section_identifier);
                va
            });
        }
        
        // Update mappings
        new_section._update_mappings_section();
        // And append
        let section_identifier = new_section.section_identifier;
        inner.sections.push_front(new_section);
        // And return
        AllocationSection::new(Arc::clone(self),section_identifier)
    }
    
    /// Map this into virtual memory in the given allocator
    pub fn map_virtual<PFA:PageFrameAllocator+Send+Sync+'static>(self: &Arc<Self>, allocator: &LockedPageAllocator<PFA>, virt_mode: VirtualAllocationMode, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        let mut inner = self.0.lock(); let n_sections = inner.sections.len();
        // Sum size and determine split positions
        let mut total_size = 0;
        let mut split_lengths = Vec::<BackingSize>::with_capacity(n_sections);  // final one isn't used but eh
        for section in inner.sections.iter() {
            total_size += section.size.get();
            split_lengths.push(section.size);
        }
        let _=split_lengths.pop();  // Pop the final one as it isn't needed
        
        // Allocate one big block of [SIZE]
        let phys_addr = inner.sections[0].backing.get_addr();
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
            (lhs, remainder) = remainder.split(split_size.get());
            debug_assert!(lhs.size() == split_size.get());
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
        for (allocation,section) in section_allocs.into_iter().zip(inner.sections.iter()) {
            let (va, _) = VirtualAllocation::new(
                Box::new(allocation) as Box<dyn AnyPageAllocation>,
                flags,
                Arc::downgrade(self), index, section.section_identifier,
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
/// A newtype wrapper representing a given section in the unified allocation
/// This is not tied to the given section - drop()ing will not de-allocate the section (as long as another Arc<> to the CombinedAllocation still exists), however you will no longer be able to operate on it
pub struct AllocationSection {
    allocation: Arc<CombinedAllocation>,
    section_identifier: usize,
}
impl !Clone for AllocationSection {}
impl AllocationSection {
    fn new(allocation: Arc<CombinedAllocation>, section_identifier: usize) -> Self {
        Self { allocation, section_identifier }
    }
    
    pub fn allocation(&self) -> &Arc<CombinedAllocation> {
        &self.allocation
    }
    pub fn section_identifier(&self) -> usize {
        self.section_identifier
    }
    
    fn allocation_inner(&self) -> YMutexGuard<'_,CombinedAllocationInner> {
        core::ops::Deref::deref(&self.allocation).0.lock()
    }
    fn segment_inner(&self) -> Option<crate::sync::MappedYMutexGuard<'_,CombinedAllocationSegment>> {
        YMutexGuard::try_map(self.allocation_inner(), |inner|inner._find_section_with_identifier_mut(self.section_identifier)).ok()
    }
    
    /// Split a section into two new sections, with new identifiers
    /// The lower section is guaranteed to contain at least `mid` bytes, though this may be rounded based on page alignment
    /// Returns Err if the section does not exist. Otherwise, returns the two section IDs
    pub fn split(self, mid: BackingSize) -> Result<(Self,Self),()> {
        let mut inner = self.allocation_inner();
        let section_idx = inner._find_section_with_identifier_index(self.section_identifier).ok_or(())?;
        
        let mut old_section = inner.sections.remove(section_idx).unwrap();
        let left_size = BackingSize::new(core::alloc::Layout::from_size_align(mid.get(), MIN_PAGE_SIZE).unwrap().pad_to_align().size()).unwrap();
        let right_size = BackingSize::new(old_section.size.get()-left_size.get()).unwrap();
        
        // Split backing allocation
        let old_backing = core::mem::replace(&mut old_section.backing, AllocationBacking::new(AllocationBackingMode::UninitMem,old_section.size));  // (take, leaving a useless allocation in its place)
        let (lhs_backing, rhs_backing) = old_backing.split(left_size);
        let rhs_backing = rhs_backing.unwrap();  // rhs_backing is None if mid >= total size
        
        let mut left_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            backing: lhs_backing,
            
            size: left_size,
            section_identifier: inner._get_new_section_identifier(),
        };
        let mut right_section = CombinedAllocationSegment {
            vmem: vec_of_non_clone![None;inner.available_virt_slots.len()],
            backing: rhs_backing,
            
            size: right_size,
            section_identifier: inner._get_new_section_identifier(),
        };
        let left_identifier = left_section.section_identifier;
        let right_identifier = right_section.section_identifier;
        
        // Split virtual memory allocations
        for (index, valloc) in old_section.vmem.drain(0..).enumerate().filter_map(|(i,o)|o.map(|x|(i,x))) {
            let VirtualAllocation { allocation, flags, .. } = valloc;
            let (left_allocation, right_allocation) = allocation.split_dyn(left_size.get());
            debug_assert!(left_allocation.size()==left_size.get());
            debug_assert!(right_allocation.size()==right_size.get());
            
            let (la,_) = VirtualAllocation::new(
                left_allocation, flags.clone(), Arc::downgrade(&self.allocation), index, left_section.section_identifier,
            );
            left_section.vmem[index] = Some(la);
            let (ra, _) = VirtualAllocation::new(
                right_allocation, flags.clone(), Arc::downgrade(&self.allocation), index, right_section.section_identifier,
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
        Ok((
            Self::new(Arc::clone(&self.allocation),left_identifier),
            Self::new(Arc::clone(&self.allocation),right_identifier),
        ))
    }
    /// Deallocate this section. Only succeeds if this is at the edge of the allocation, as allocations cannot have holes.
    pub fn deallocate(self) -> Result<(),Self> {
        let mut inner = self.allocation_inner();
        let section_idx = inner._find_section_with_identifier_index(self.section_identifier).unwrap();
        
        let this_section = if section_idx == 0 { inner.sections.pop_front().unwrap() }
                            else if section_idx == inner.sections.len()-1 { inner.sections.pop_back().unwrap() }
                            else { drop(inner); return Err(self); };
        
        // We have now removed ourselves from the allocation
        // We drop the section to deallocate it from memory (RAII go brrrrrr)
        drop(this_section);
        Ok(())
    }
    
    /// Load a section from swap into physical memory
    pub fn swap_in(&self) -> Result<(),SwapInError> {
        let mut section = self.segment_inner().ok_or(SwapInError::SectionNotFound)?;
        
        // Swap in
        section.backing.swap_in().map_err(|x|SwapInError::LoadError(x))?;
        
        // Update mappings
        section._update_mappings_section();
        // Done :)
        Ok(())
    }
    
    /// Overwrite a section with a new backing type, wiping the previous memory allocated to it and discarding its contents
    /// (the size in new_request must equal the present size of the section)
    /// true = success (overwrite ok), false = failure (previous state remains)
    pub fn overwrite_type(&self, new_request: AllocationBackingRequest) -> bool {
        let Some(mut section) = self.segment_inner() else { return false; };
        
        debug_assert!(section.size == new_request.get_size());
        // Allocate new backing
        let (new_backing,_) = AllocationBacking::new_from_request(new_request);
        // Overwrite previous allocation
        section.backing = new_backing;
        
        // Update mappings
        section._update_mappings_section();
        // Done :)
        true
    }
}
pub enum SwapInError {
    /// Backing load error
    LoadError(BackingLoadError),
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
    fn combined_allocation_inner(&self) -> YMutexGuard<'_,CombinedAllocationInner> {
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

// impl super::alloc_util::AnyAllocatedStack for VirtAllocationGuard {
//     // assumes the stack grows downwards
//     fn bottom_vaddr(&self) -> usize {
//         self.end_addr()
//     }
//     fn expand(&mut self, bytes: usize) -> bool {
//         let start = self.start_addr();
//         self.combined_allocation().expand_downwards(AllocationBackingRequest::UninitPhysical { size: bytes });  // on success, this will change our start_addr
//         start != self.start_addr()
//     }
// }
impl core::fmt::Debug for VirtAllocationGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VirtAllocationGuard [ {:x}..{:x} ]", self.start_addr(), self.end_addr())
    } 
}

pub struct AllocatedStack {  // assumes stack grows downwards
    virt_stack: VirtAllocationGuard,
    
    guard_page: AllocationSection,
    guard_size: BackingSize,
}
impl AllocatedStack {
    pub fn alloc_new<PFA:PageFrameAllocator+Send+Sync+'static>(stack_size: BackingSize, guard_size: BackingSize, alloc_flags: AllocationFlags, virt_allocator: &LockedPageAllocator<PFA>, virt_mode: VirtualAllocationMode, virt_flags: VMemFlags) -> Option<Self> {
        // Map into physical memory
        let sections = CombinedAllocation::new_from_requests(alloc::vec![
            // Guard Page
            AllocationBackingRequest::GuardPage { gptype: GuardPageType::StackLimit, size: guard_size },
            // Stack
            AllocationBackingRequest::UninitPhysical { size: stack_size },
        ], alloc_flags);
        let guard_page = sections.into_iter().nth(0)?;
        // Map into virtual memory
        let virt_allocation = guard_page.allocation().map_virtual(virt_allocator, virt_mode, virt_flags)?;
        // Ok :)
        Some(Self {
            virt_stack: virt_allocation,
            guard_page: guard_page,
            guard_size: guard_size,
        })
    }
    
    pub fn bottom_vaddr(&self) -> usize {
        self.virt_stack.end_addr()
    }
    
    pub fn expand(&mut self, size_bytes: usize) -> bool {
        let old_start = self.virt_stack.start_addr();  // (the only way to check if our specific virtual allocation has expanded is to check its start address)
        
        // Step 1: Allocate the new expansion + guard page
        let extra_expansion_size = size_bytes.saturating_sub(self.guard_size.get());  // We consume the existing guard page as part of the expansion
        let total_expansion_size = BackingSize::new(extra_expansion_size + self.guard_size.get()).unwrap();  // X+guard_size is guaranteed to be >0 since guard_size > 0
        let expanded_section = self.virt_stack.combined_allocation().expand_downwards(AllocationBackingRequest::Reservation { size: total_expansion_size });
        // Split new expansion into expansion + guard page
        let (expansion, new_guard) = if let Some(extra_expansion_size) = BackingSize::new(extra_expansion_size) {
            // Split expanded section into two
            let (guard, expansion) = expanded_section.split(self.guard_size).unwrap();
            expansion.overwrite_type(AllocationBackingRequest::UninitPhysical { size: extra_expansion_size });  // map expansion into memory
            (Some(expansion), guard)
        } else {
            // No extra expansion needed
            (None, expanded_section)
        };
        
        // Check for success - vmem addresses should be updated now if this was successful./
        // (if it wasn't, we must undo)
        let success = self.virt_stack.start_addr() != old_start;
        if !success {
            new_guard.deallocate().ok().expect("Deallocation failed!");
            if let Some(expansion) = expansion { expansion.deallocate().ok().expect("Deallocation failed!"); }
            return false;
        }
        
        // Step 2: Switch guards, and map the old guard as available stack space
        new_guard.overwrite_type(AllocationBackingRequest::GuardPage { gptype: GuardPageType::StackLimit, size: self.guard_size });
        let old_guard = core::mem::replace(&mut self.guard_page, new_guard);
        old_guard.overwrite_type(AllocationBackingRequest::UninitPhysical { size: self.guard_size });
        
        // Success
        true
    }
}

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
struct AbsentPagesItemA {
    allocation: Weak<CombinedAllocation>,
    virt_allocation_index: usize,
    section_identifier: usize,
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<(),AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();
}