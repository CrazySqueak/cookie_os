
use core::ops::{Deref,DerefMut};
use alloc::boxed::Box;
use alloc::vec::Vec; use alloc::vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,PAGE_ALIGN,PageAlignedUsize};
use super::physical::{PhysicalMemoryAllocation,palloc};
use bitflags::bitflags;
use crate::sync::{YMutex,YMutexGuard,ArcYMutexGuard};

use super::paging::{global_pages::KERNEL_PTABLE,strategy::KALLOCATION_KERNEL_GENERALDYN,strategy::PageAllocationStrategies};

macro_rules! vec_of_non_clone {
    [$item:expr ; $count:expr] => {
        Vec::from_iter((0..$count).map(|_|$item))
    }
}

#[derive(Clone,Copy,Debug)]
pub enum GuardPageType {
    StackLimit = 0xF47B33F,  // Fat Beef
    NullPointer = 0x4E55_4C505452,  // "NULPTR"
}
pub type BackingSize = PageAlignedUsize;
/// A request for allocation backing
pub enum AllocationBackingRequest {
    UninitPhysical { size: BackingSize },
    ZeroedPhysical { size: BackingSize },
    /// Similar to UninitPhysical, but isn't automatically allocated in physical memory. Instead, it's initialised as an UninitMem.
    Reservation { size: BackingSize },
    
    GuardPage { gptype: GuardPageType, size: BackingSize },
}
impl AllocationBackingRequest {
    pub fn get_size(&self) -> BackingSize {
        match *self {
            Self::UninitPhysical{size} => size,
            Self::ZeroedPhysical{size} => size,
            Self::Reservation{size} => size,
            Self::GuardPage{size,..} => size,
        }
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
pub struct AllocationBacking {
    mode: AllocationBackingMode,
    size: BackingSize,
}
impl AllocationBacking {
    pub(self) fn new(mode: AllocationBackingMode, size: BackingSize) -> Self {
        Self { mode, size }
    }
    
    fn _palloc(size: PageAlignedUsize) -> Option<PhysicalMemoryAllocation> {
        palloc(size)
    }
    /// Returns (self,true) if the request was fulfilled immediately. Returns (self,false) if it couldn't, and must be swapped in later when more RAM is available
    pub fn new_from_request(request: AllocationBackingRequest) -> (Self,bool) {
        let size = request.get_size();
        match request {
            AllocationBackingRequest::GuardPage { gptype, .. } => (Self::new(AllocationBackingMode::GuardPage(gptype),size),true),
            AllocationBackingRequest::Reservation { size } => (Self::new(AllocationBackingMode::UninitMem,size),true),
            
            AllocationBackingRequest::UninitPhysical { .. } => {
                match Self::_palloc(size) {
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
    
    /// Split this allocation into two. One of `midpoint` bytes and one of the remainder
    /// midpoint MUST be < self.size
    pub fn split(self, midpoint: BackingSize) -> (Self,Self) {
        //if midpoint >= self.size { return (self,None); }
        debug_assert!(midpoint < self.size);
        let midpoint: usize = midpoint.get();
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
        (Self::new(lhs_mode,BackingSize::new_checked(lhs_size).unwrap()),
         Self::new(rhs_mode,BackingSize::new_checked(rhs_size).unwrap()))
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
                 let phys_alloc = Self::_palloc(self.size).ok_or(BackingLoadError::PhysicalAllocationFailed)?;
                 match self.mode {
                     AllocationBackingMode::PhysMem(_) | AllocationBackingMode::PhysMemShared{..} | AllocationBackingMode::GuardPage(_) => unreachable!(),
                     
                     AllocationBackingMode::UninitMem => {},  // uninit mem can be left as-is
                     AllocationBackingMode::Zeroed => {  // zeroed and other backing modes must be mapped into vmem and initialised
                        // Map into vmem and obtain pointer
                        let vmap = KERNEL_PTABLE.allocate(self.size, KALLOCATION_KERNEL_GENERALDYN).expect("How the fuck are you out of virtual memory???");
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
                  combined_alloc: Weak<CALocked>, virt_index: usize, apt_allocation_offset: isize) -> (Self,AbsentPagesHandleB) {
        // Populate an absent_pages_table entry
        let ate = ABSENT_PAGES_TABLE.create_new_descriptor();
        let apth = ate.commit(AbsentPagesItemA {
            allocation: combined_alloc,
            virt_allocation_index: virt_index,
            allocation_offset: apt_allocation_offset,
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
    
    /// mid MUST be < size
    pub fn split(self, mid: BackingSize) -> (Self,Self) {
        debug_assert!(mid < self.size());
        let Self { allocation, flags, absent_pages_table_handle: apth } = self;
        let apt_data = apth.get_a();
        let apt_alloc_weak = &apt_data.allocation;
        let virt_idx = apt_data.virt_allocation_index;
        let lhs_apt_offset = apt_data.allocation_offset;
        let rhs_apt_offset = lhs_apt_offset + (mid.get() as isize);
        
        let (allocation_left, allocation_right) = allocation.split_dyn(mid);
        (Self::new(allocation_left, flags, Weak::clone(apt_alloc_weak), virt_idx, lhs_apt_offset).0,
         Self::new(allocation_right, flags, Weak::clone(apt_alloc_weak), virt_idx, rhs_apt_offset).0)
    }
    
    pub fn start_addr(&self) -> usize {
        self.allocation.start()
    }
    pub fn size(&self) -> PageAlignedUsize {
        self.allocation.size()
    }
    pub fn end_addr(&self) -> usize {
        self.allocation.end()
    }
}
#[derive(Debug,Clone,Copy)]
pub enum VirtualAllocationMode {
    Dynamic { strategy: PageAllocationStrategies<'static> },
    // TODO OffsetMapped { offset: usize },
    FixedVirtAddr { addr: usize },
}
/// Lookup an "absent page" data item in the ABSENT_PAGES_TABLE
/// Returns the CombinedAllocationBacking API object, the offset in memory relative to the "base", and the index of the virtual allocation.
pub fn lookup_absent_id(absent_id: usize) -> Option<AbsentLookupResult> {
    let apth_a = ABSENT_PAGES_TABLE.acquire_a(absent_id.try_into().unwrap()).ok()?;
    let apt_item_a = apth_a.get_a();
    let combined_alloc = CombinedAllocationBacking(Weak::upgrade(&apt_item_a.allocation)?);
    let virt_index = apt_item_a.virt_allocation_index;
    let allocation_offset = apt_item_a.allocation_offset;
    
    Some(AbsentLookupResult { backing_api: combined_alloc, alloc_offset: allocation_offset, virt_index })
}
pub struct AbsentLookupResult {
    backing_api: CombinedAllocationBacking,
    alloc_offset: isize,
    virt_index: usize,
}

bitflags! {
    #[derive(Clone,Copy,Debug)]
    pub struct AllocationFlags : u32 {
        /// This may not be un-mapped from physical memory, or moved around within physical memory
        const STICKY = 1<<0;
    }
}
// NOTE: CombinedAllocation must ALWAYS be locked BEFORE any page allocators (if you are nesting the locks, which isn't recommended but often necessary)!!

enum CASegVirtAllocSlot {
    /// An empty slot
    Empty,
    /// Occupied - failed allocation
    Failure,
    /// Occupied - successful allocation
    Allocation(VirtualAllocation),
}
impl CASegVirtAllocSlot {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            _ => false,
        }
    }
    pub fn is_filled(&self) -> bool {
        !self.is_empty()
    }
    
    pub fn replace(&mut self, success: VirtualAllocation) -> Self {
        core::mem::replace(self, Self::Allocation(success))
    }
    pub fn replace_with_failed(&mut self) -> Self {
        core::mem::replace(self, Self::Failure)
    }
    pub fn take(&mut self) -> Self {
        core::mem::replace(self, Self::Empty)
    }
    
    pub fn allocation_ref(&self) -> Option<&VirtualAllocation> {
        match self {
            Self::Allocation(alloc) => Some(alloc),
            _ => None,
        }
    }
}
// TODO
/// Each "segment" is a logical division in the "backing", being tied to a given allocation backing.
/// Segments have a fixed size, but may be split and (possibly merged?).
pub struct CombinedAllocationSegment {
    vmem: Vec<CASegVirtAllocSlot>,  // vmem is placed first in drop order so it's dropped/cleared before the backing is
    backing: AllocationBacking,
}
impl CombinedAllocationSegment {
    pub(self) fn new(backing: AllocationBacking, vmem_slot_count: usize) -> Self {
        Self {
            vmem: vec_of_non_clone![CASegVirtAllocSlot::Empty;vmem_slot_count],
            backing: backing,
        }
    }
    
    pub fn get_size(&self) -> BackingSize {
        self.backing.get_size()
    }
    
    // SWAPPING
    /// Swap the backing into memory
    pub fn swap_in(&mut self) -> Result<BackingLoadSuccess,BackingLoadError> {
        let result = self.backing.swap_in();
        self.remap_all_pages();
        result
    }
    
    /// Replace the backing with a new backing, discarding any previously written data to the old backing
    pub(self) fn overwrite_backing(&mut self, new_backing: AllocationBacking) {
        debug_assert!(new_backing.get_size() == self.backing.get_size());
        let old = core::mem::replace(&mut self.backing, new_backing);
        
        // Make sure to remap all pages before dropping the old backing
        self.remap_all_pages();
        drop(old);
    }
    
    // SPLITTING/MERGING
    /// Midpoint MUST be < self.size
    pub fn split(self, mid: BackingSize) -> (Self,Self) {
        //if mid >= self.get_size() { return (self, None); }
        debug_assert!(mid < self.get_size());
        let Self { backing, vmem } = self;
        
        // Split the backing
        let (lhs_backing, rhs_backing) = backing.split(mid);
        
        // Split the vmem allocations
        let mut lhs_vmem: Vec<CASegVirtAllocSlot> = vec_of_non_clone![CASegVirtAllocSlot::Empty;vmem.len()];
        let mut rhs_vmem: Vec<CASegVirtAllocSlot> = vec_of_non_clone![CASegVirtAllocSlot::Empty;vmem.len()];
        for (slot, old_alloc) in vmem.into_iter().enumerate() {
            match old_alloc {
                CASegVirtAllocSlot::Allocation(old_alloc) => {
                    let (lhs_alloc, rhs_alloc) = old_alloc.split(mid);
                    lhs_vmem[slot].replace(lhs_alloc);
                    rhs_vmem[slot].replace(rhs_alloc);
                },
                CASegVirtAllocSlot::Failure => {
                    lhs_vmem[slot] = CASegVirtAllocSlot::Failure;
                    rhs_vmem[slot] = CASegVirtAllocSlot::Failure;
                },
                CASegVirtAllocSlot::Empty => {}, // We only process extant vmem allocations. Empty slots are skipped.
            }
        }
        
        // Done :)
        let lhs = Self { backing: lhs_backing, vmem: lhs_vmem }; lhs.remap_all_pages();
        let rhs = Self { backing: rhs_backing, vmem: rhs_vmem }; rhs.remap_all_pages();
        (lhs, rhs)
    }
    
    // VMEM MAPPING
    pub(self) fn map_vmem(&mut self, slot: usize, valloc: VirtualAllocation) {
        while slot < self.vmem.len() { self.vmem.push(CASegVirtAllocSlot::Empty); }  // Ensure vmem has the requested slot
        
        debug_assert!(valloc.size()==self.get_size());  // ensure that the sizes match
        let prev = self.vmem[slot].replace(valloc);
        debug_assert!(prev.is_empty());  // ensure we're not overwriting an in-use slot
        
        // Remap
        self.remap_pages(slot);
    }
    pub(self) fn clear_vmem(&mut self, slot: usize) {
        let old = self.vmem[slot].take();
        debug_assert!(old.is_filled());  // ensure the slot was actually in use
        drop(old);  // clarity: manually drop `old` to ensure that the page table is cleared of the old mappings
    }
    
    /// Update the page table for the given virtual allocation to match any changes that have been made
    fn remap_pages(&self, slot: usize) {
        let Some(valloc) = (try{ self.vmem.get(slot)?.allocation_ref()? }) else { return };
        self._remap_pages_inner(valloc)
    }
    /// Update the page table for all virtual allocations, with respect to this particular segment
    fn remap_all_pages(&self) {
        for valloc in self.vmem.iter().map(CASegVirtAllocSlot::allocation_ref).flatten() {
            self._remap_pages_inner(valloc)
        }
    }
    fn _remap_pages_inner(&self, valloc: &VirtualAllocation) {
        match self.backing.get_addr() {
            Some(addr) => valloc.map(addr),
            None => valloc.set_absent(),
        }
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct CASegmentIndex { section: usize, segment: usize }
pub struct CASegmentWriter<'r> {
    inner: &'r mut CombinedAllocationInner,
    index: CASegmentIndex,
}
impl<'r> CASegmentWriter<'r> {
    /// Select a different segment, using its index
    pub fn with_index(self, index: CASegmentIndex) -> Self {
        self.inner.get_segment_mut(index)
    }
    
    /// Split a segment in two. Returns the left-hand-side, and the index of the right-hand-side
    pub fn split(self, mid: BackingSize) -> (Self,Option<CASegmentIndex>) {
        if mid >= self.get_size() { return (self,None); }  // mid must be < segment.size
        
        // Use swap_remove for magic
        // (we set things right afterwards)
        let Self { inner:self_inner, index:self_index } = self;  // destructure self to avoid accidental derefs
        let segments = &mut self_inner.sections[self_index.section].segments;
        let segment = segments.swap_remove(self_index.segment);
        let (lhs,rhs) = segment.split(mid);
        // Push lhs to the end, and swap it with the previously swapped element
        let end_idx = segments.len();
        segments.push(lhs);
        segments.swap(self_index.segment, end_idx-1);  // swap the swapped element with our LHS
        segments.insert(self_index.segment+1, rhs);  // insert rhs directly after lhs
        
        // Return new handles
        let rhs_index = CASegmentIndex { section: self_index.section, segment: self_index.segment+1 };
        (self_inner.get_segment_mut(self_index), Some(rhs_index))
    }
}
impl Deref for CASegmentWriter<'_> {
    type Target = CombinedAllocationSegment;
    fn deref(&self) -> &Self::Target {
        self.inner.get_segment(self.index)
    }
}
impl DerefMut for CASegmentWriter<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner._get_segment_mut_inner(self.index)
    }
}

/// Each "section" is a logical division in the contract. For example, working memory and the guard page would be two different sections
/// Sections may be transparently subdivided into multiple segments by the memory manager as it sees fit.
/// Sections may not be resized, but may be split/merged.
pub struct CombinedAllocationSection {
    segments: Vec<CombinedAllocationSegment>,
    
    /// Section identifier - a unique value assigned to each section within an allocation. This does not change until the section itself is merged/split, even if other sections are added/removed.
    section_identifier: CASectionIdentifier,
    /// Total size in bytes
    total_size: BackingSize,
    /// Offset from the allocation "base" in vmem
    offset: isize,
}
impl CombinedAllocationSection {
    pub(self) fn new(context: &mut CombinedAllocationContext, offset: isize, backing: AllocationBacking, vmem_slot_count: usize) -> Self {
        Self {
            offset: offset,
            total_size: backing.size,
            section_identifier: context.take_next_section_identifier(),
            segments: vec![CombinedAllocationSegment::new(backing, vmem_slot_count)],
        }
    }
    
    fn calculate_size(&self) -> BackingSize {
        let total = self.segments.iter().fold(0usize, |a,x| a+x.backing.size.get());
        BackingSize::new(total)
    }
    pub(self) fn recalculate_size(&mut self) {
        self.total_size = self.calculate_size()
    }
    pub fn get_size(&self) -> BackingSize {
        self.total_size
    }
    pub fn get_offset(&self) -> isize {
        self.offset
    }
    pub fn get_section_identifier(&self) -> CASectionIdentifier {
        self.section_identifier
    }
    
    // BACKING TYPES
    pub fn overwrite_backing(&mut self, new_backing: AllocationBacking) {
        debug_assert!(self.get_size() == new_backing.get_size());
        
        // We have to split the backing into pieces for each segment
        let mut remainder = new_backing;
        for segment in self.segments.iter_mut().rev().skip(1).rev() {  // skip the final segment
            let (lhs, rhs) = remainder.split(segment.get_size());
            segment.overwrite_backing(lhs);
            remainder = rhs;
        }
        // Handle the last one
        self.segments.last_mut().unwrap().overwrite_backing(remainder);
    }
    
    // SPLITTING/MERGING
    // todo: splitting
    
    /// Merge the two sections together. Backing memory is left as-is unless overwritten afterwards.
    /// SAFETY: The caller must ensure that both sections are directly adjacent to each other, with no gaps, and with lhs occupying lower addresses than rhs.
    pub(self) unsafe fn merge(context: &mut CombinedAllocationContext, lhs: Self, rhs: Self) -> Self {
        let Self { segments: lhs_segments, total_size: lhs_size, offset: lhs_offset, .. } = lhs;
        let Self { segments: rhs_segments, total_size: rhs_size, offset: rhs_offset, .. } = rhs;
        
        debug_assert!(lhs_offset+(lhs_size.get() as isize) == rhs_offset);  // assert that sections are next to eachother in memory
        
        let sum_sizes = BackingSize::new(lhs_size.get()+rhs_size.get());
        let mut segments = lhs_segments;
        segments.reserve(rhs_segments.len());
        for segment in rhs_segments { segments.push(segment); }
        
        Self { segments, total_size: sum_sizes, offset: lhs_offset, section_identifier: context.take_next_section_identifier() }
    }
    
    // VMEM MAPPING
    /// Add a new vmem mapping in the given slot
    /// Takes one big VirtualAllocation and splits it into each piece
    pub(self) fn map_vmem(&mut self, slot: usize, allocation: VirtualAllocation) {
        debug_assert!(allocation.size() == self.total_size);
        debug_assert!(allocation.absent_pages_table_handle.get_a().allocation_offset == self.offset);
        
        split_and_map_vmem(self.segments.iter_mut(), allocation,
                           |seg|seg.get_size(), |seg,valloc|seg.map_vmem(slot,valloc));
        // All done :)
    }
    pub(self) fn _map_vmem_failure(&mut self, slot: usize) {
        for segment in self.segments.iter_mut() {
            let old = segment.vmem[slot].replace_with_failed();
            debug_assert!(old.is_empty());
        }
    }
    pub(self) fn clear_vmem(&mut self, slot: usize) {
        for segment in self.segments.iter_mut() {
            segment.clear_vmem(slot)
        }
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub struct CASectionIndex(usize);
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct CASectionIdentifier(usize);
pub struct CASectionWriter<'r> {
    inner: &'r mut CombinedAllocationInner,
    index: CASectionIndex,
}
impl<'r> CASectionWriter<'r> {
    /// Select a different section, using its index
    pub fn with_index(self, index: CASectionIndex) -> Self {
        self.inner.get_section_mut(index)
    }
    /// Select a different section, using its identifier
    pub fn with_identifier(self, identifier: CASectionIdentifier) -> Option<Self> {
        let index = self.inner.get_section_index_from_identifier(identifier)?;
        Some(self.with_index(index))
    }
    
    /// Merge two sections together. This section must be directly below the next one in memory, with no gaps.
    /// Returns Ok(merged) on success, Err(self) on failure
    pub fn merge(self, rhs: CASectionIdentifier) -> Result<Self,Self> {
        let Some(rhs) = self.inner.get_section_index_from_identifier(rhs) else { return Err(self); };
        let lhs_offset = self.get_offset();
        let lhs_size = self.get_size().get() as isize;
        let rhs_offset = self.inner.get_section(rhs).get_offset();
        if lhs_offset+lhs_size != rhs_offset { return Err(self); }  // Not next to each other
        
        // Precondition has been validated - we can safely merge
        let Self { inner:self_inner, index:self_index } = self;  // destructure self to avoid accidental derefs
        let sections = &mut self_inner.sections;
        let mut sections_to_merge = sections.drain(self_index.0..self_index.0+2);
        let lhs = sections_to_merge.next().unwrap();
        let rhs = sections_to_merge.next().unwrap();
        drop(sections_to_merge);
        let merged = unsafe { CombinedAllocationSection::merge(&mut self_inner.context, lhs, rhs) };  // safety: we've validated the preconditions
        
        // Add merged back to list
        sections.insert(self_index.0, merged);
        // And return a new writer
        Ok(self_inner.get_section_mut(self_index))
    }
}
impl Deref for CASectionWriter<'_> {
    type Target = CombinedAllocationSection;
    fn deref(&self) -> &Self::Target {
        self.inner.get_section(self.index)
    }
}
impl DerefMut for CASectionWriter<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner._get_section_mut_inner(self.index)
    }
}

struct CombinedAllocationContext {
    /// The section identifier for the next section to be created
    next_section_identifier: usize,
    /// Which virtual allocation slots are free
    free_virt_slots: Vec<bool>,
}
impl CombinedAllocationContext {
    pub(self) fn take_next_section_identifier(&mut self) -> CASectionIdentifier {
        let x = self.next_section_identifier;
        self.next_section_identifier += 1;
        CASectionIdentifier(x)
    }
}
struct CombinedAllocationInner {
    sections: VecDeque<CombinedAllocationSection>,
    context: CombinedAllocationContext,
    total_size: BackingSize,
}
impl CombinedAllocationInner {
    fn calculate_size(&self) -> BackingSize {
        let total = self.sections.iter().fold(0usize, |a,x| a+x.get_size().get());
        BackingSize::new(total)
    }
    pub(self) fn recalculate_size(&mut self) {
        self.sections.iter_mut().for_each(|sec|sec.recalculate_size());
        self.total_size = self.calculate_size();
    }
    pub fn get_size(&self) -> BackingSize {
        self.total_size
    }
    
    fn get_section_index_from_identifier(&self, identifier: CASectionIdentifier) -> Option<CASectionIndex> {
        Some(CASectionIndex(self.sections.iter().position(|x|x.get_section_identifier()==identifier)?))
    }
    fn get_section(&self, index: CASectionIndex) -> &CombinedAllocationSection {
        &self.sections[index.0]
    }
    fn _get_section_mut_inner(&mut self, index: CASectionIndex) -> &mut CombinedAllocationSection {
        &mut self.sections[index.0]
    }
    fn get_section_mut(&mut self, index: CASectionIndex) -> CASectionWriter {
        CASectionWriter { inner: self, index: index }
    }
    fn sections_iter(&self) -> impl Iterator<Item=&CombinedAllocationSection> {
        self.sections.iter()
    }
    fn sections_iter_mut(&mut self) -> impl Iterator<Item=&mut CombinedAllocationSection> {
        self.sections.iter_mut()
    }
    fn section_indexes_iter(&self) -> impl Iterator<Item=CASectionIndex> {
        (0..self.sections.len()).map(|sec_id| CASectionIndex(sec_id))
    }
    
    fn get_segment(&self, index: CASegmentIndex) -> &CombinedAllocationSegment {
        &self.sections[index.section].segments[index.segment]
    }
    fn _get_segment_mut_inner(&mut self, index: CASegmentIndex) -> &mut CombinedAllocationSegment {
        &mut self.sections[index.section].segments[index.segment]
    }
    fn get_segment_mut(&mut self, index: CASegmentIndex) -> CASegmentWriter {
        CASegmentWriter { inner: self, index: index }
    }
    fn segments_iter(&self) -> impl Iterator<Item=&CombinedAllocationSegment> {
        self.sections.iter().flat_map(|sec|sec.segments.iter())
    }
    fn segments_iter_mut(&mut self) -> impl Iterator<Item=&mut CombinedAllocationSegment> {
        self.sections.iter_mut().flat_map(|sec|sec.segments.iter_mut())
    }
    fn segment_indexes_iter(&self) -> impl Iterator<Item=CASegmentIndex> + '_ {
        self.section_indexes_iter().flat_map(|sec_id|(0..self.get_section(sec_id).segments.len()).map(move|seg_id| CASegmentIndex { section: sec_id.0, segment: seg_id }))
    }
    
    /// Push a new section to the lower end of this contract
    fn push_new_section_lower(&mut self, backing: AllocationBacking, calloc: &Arc<CALocked>) -> CASectionWriter {
        // Calculate size and offset
        let size = backing.get_size();
        let offset = self.sections[0].get_offset() - (size.get() as isize);
        let vmem_slot_count = self.context.free_virt_slots.len();
        // Create new section and push it
        let section = CombinedAllocationSection::new(&mut self.context, offset, backing, vmem_slot_count);
        self.sections.push_front(section);
        
        // (attempt) to map into vmem - extending existing allocations
        // we look at the earliest allocation (after us) to determine how we extend the allocations/etc.
        debug_assert!(self.sections[1].segments.len() >= 1);  // prev earliest allocation: only the first segment is relevant
        let next_segment = &self.sections[1].segments[0];
        // Because VecDeque doesn't have a method for borrowing two elements at once, we have to do this in two passes
        // Pass 1: Attempt allocation
        let mut new_allocations = Vec::<(usize,Option<VirtualAllocation>)>::with_capacity(vmem_slot_count);  // some = success, none = failure, not present = empty
        for (slot_index,old_slot) in next_segment.vmem.iter().enumerate() {
            match old_slot {
                CASegVirtAllocSlot::Empty => continue,  // ignore empty allocs
                CASegVirtAllocSlot::Failure => new_allocations.push((slot_index,None)),  // extend failures along
                CASegVirtAllocSlot::Allocation(alloc) => {
                    match alloc.allocation.alloc_downwards_dyn(size) {
                        None => new_allocations.push((slot_index,None)),
                        Some(new_alloc) => {
                            let (virt,_) = VirtualAllocation::new(
                                new_alloc, alloc.flags, Arc::downgrade(&calloc), slot_index, offset,
                            );
                            new_allocations.push((slot_index,Some(virt)));
                        },
                    };
                },
            }
        }
        // Pass 2: Map allocated vmem
        let section = &mut self.sections[0];
        for (slot_index,allocation_result) in new_allocations {
            match allocation_result {
                None => section._map_vmem_failure(slot_index),
                Some(virt) => section.map_vmem(slot_index, virt),
            }
        }
        
        // All gucci
        self.get_section_mut(CASectionIndex(0))
    }
    
    /// Take ownership of a free vmem slot
    fn pick_vmem_slot(&mut self) -> usize {
        // Select a slot
        let index_result = self.context.free_virt_slots.iter().position(|s|*s).ok_or(self.context.free_virt_slots.len());
        let slot = match index_result {
            Ok(index) => {self.context.free_virt_slots[index]=false;index},
            Err(new_index) => {self.context.free_virt_slots.push(false);new_index},
        };
        slot
    }
    /// Add a new vmem mapping
    /// Takes one big VirtualAllocation and splits it into each piece
    fn map_vmem(&mut self, allocation: VirtualAllocation, slot: usize) {
        debug_assert!(self.get_size() == allocation.size());
        
        // Allocate
        split_and_map_vmem(self.sections.iter_mut(), allocation,
                           |sec|sec.get_size(), |sec,valloc|sec.map_vmem(slot,valloc));
    }
    fn clear_vmem(&mut self, slot: usize) {
        // Free slot
        let old = core::mem::replace(&mut self.context.free_virt_slots[slot],false);
        debug_assert!(old);
        // Free allocations
        for segment in self.sections.iter_mut() {
            segment.clear_vmem(slot)
        }
    }
}
type CALocked = YMutex<CombinedAllocationInner>;

fn split_and_map_vmem<'i,I:'i>(mut iterator: impl DoubleEndedIterator<Item=&'i mut I>, allocation: VirtualAllocation,
                         get_size: impl Fn(&I)->BackingSize, map_vmem: impl Fn(&mut I,VirtualAllocation)) {
    let final_item = iterator.next_back().unwrap();  // the final one takes the remainder, so it's handled separately
    // Chop off a piece for each item, one by one
    let mut remainder = allocation;
    for item in iterator {
        let (lhs,rhs) = remainder.split(get_size(item));
        map_vmem(item, lhs);
        remainder = rhs;
    }
    // The final item takes the remainder
    map_vmem(final_item,remainder);
    
    // This is the duplicated code this is a generalisation of V
    // // Chop off a piece for each segment, one by one
    // let mut remainder = allocation;
    // for segment in self.segments.iter_mut().rev().skip(1).rev() {  // Skip the final segment, as we handle that below
    //     let (lhs,rhs) = remainder.split(segment.get_size());
    //     segment.map_vmem(slot, lhs);
    //     remainder = rhs;
    // }
    // // The final segment was skipped above, because it simply takes the whole remainder
    // self.segments.last_mut().unwrap().map_vmem(slot, remainder);
 }

macro_rules! impl_ca_lock {
    ($name:ident,$guard:ident) => {
        impl $name {
            pub fn lock(&self) -> $guard {
                $guard(self.0.lock_arc())
            }
            pub fn try_lock(&self) -> Option<$guard> {
                self.0.try_lock_arc().map(|g|$guard(g))
            }
            pub fn is_locked(&self) -> bool {
                self.0.is_locked()
            }
        }
        impl $guard {
            pub fn allocation(&self) -> $name {
                $name(Arc::clone(ArcYMutexGuard::mutex(&self.0)))
            }
            pub fn into_allocation(self) -> $name {
                $name(ArcYMutexGuard::into_arc(self.0))
            }
        }
    }
}

/// CombinedAllocations are an abstraction between the "contract" that the allocation obeys (which vmem is for what purpose), and the "backing" (the actual memory/swap/whatever allocated to fulfill the contract).
/// While modifications to the contract entail modifying the backing to match, modifying the backing does not modify the contract.
/// In other words, how the memory is laid out / split / managed by the manager is transparent.
///
/// The contract contains the APIs for managing sections, requesting memory/guard pages/reservations/etc.
pub struct CombinedAllocationContract(Arc<CALocked>);
pub struct CombinedAllocationContractGuard(ArcYMutexGuard<CombinedAllocationInner>);
impl_ca_lock!(CombinedAllocationContract,CombinedAllocationContractGuard);
impl CombinedAllocationContract {
    pub fn new(requests: Vec<AllocationBackingRequest>) -> (Self,Vec<(CASectionIdentifier,bool)>) {  // (identifier, backing ready?)
        let mut context = CombinedAllocationContext {
            next_section_identifier: 0,
            free_virt_slots: vec![true],
        };
        let mut offset: isize = 0;
        let mut sections: VecDeque<CombinedAllocationSection> = VecDeque::with_capacity(requests.len());
        let mut section_results = Vec::with_capacity(requests.len());
        for request in requests {
            let (backing,backing_ready) = AllocationBacking::new_from_request(request);
            let section = CombinedAllocationSection::new(&mut context, offset, backing, 1);
            
            offset += section.get_size().get() as isize;
            section_results.push((section.get_section_identifier(), backing_ready));
            sections.push_back(section);
        }
        
        let this = Self(Arc::new(YMutex::new(CombinedAllocationInner {
            sections: sections,
            context: context,
            total_size: BackingSize::new(offset as usize),  // Since offset starts counting from zero, and we count from left-to-right, the final offset is equal to the total size. Huh, neat
        })));
        (this, section_results)
    }
    
    pub fn clone_ref(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
    pub fn backing(self) -> CombinedAllocationBacking {
        CombinedAllocationBacking(self.0)
    }
}
impl CombinedAllocationContractGuard {
    fn arc_ref(&self) -> &Arc<CALocked> {
        ArcYMutexGuard::mutex(&self.0)
    }
    fn downgraded_ref(&self) -> Weak<CALocked> {
        Arc::downgrade(self.arc_ref())
    }
    
    pub fn push_new_section_lower(&mut self, request: AllocationBackingRequest) -> CASectionWriter {
        let (backing, _) = AllocationBacking::new_from_request(request);
        let arc_ref = Arc::clone(self.arc_ref());
        self.0.push_new_section_lower(backing, &arc_ref)
    }
    
    fn map_vmem(&mut self, allocation: VirtualAllocation, slot: usize) -> VirtAllocationGuard {
        self.0.map_vmem(allocation, slot);
        let contract = self.allocation();
        VirtAllocationGuard { contract, virt_index: slot }
    }
    pub fn map_allocate_vmem<PFA:PageFrameAllocator+Send+Sync+'static>(&mut self, allocator: &LockedPageAllocator<PFA>, mode: VirtualAllocationMode, flags: VMemFlags) -> Option<VirtAllocationGuard> {
        // Allocate vmem
        let total_size = self.0.get_size();
        let offset: isize = self.0.sections[0].get_offset();  // N.B. FixedVirtAddr is the base address, not the start address, so we need to know the offset of the start
        let virt_allocation = match mode {
            VirtualAllocationMode::Dynamic { strategy } => allocator.allocate(total_size, strategy)?,
            VirtualAllocationMode::FixedVirtAddr { addr } => allocator.allocate_at(((addr as isize)+offset) as usize, total_size)?,
        };
        
        // Allocate slot
        let slot = self.0.pick_vmem_slot();
        
        // Build allocation
        let (valloc,_) = VirtualAllocation::new(
            Box::new(virt_allocation) as Box<dyn AnyPageAllocation>,
            flags, self.downgraded_ref(), 
            slot, offset
        );
        // Map allocation
        let guard = self.map_vmem(valloc, slot);
        // Return guard
        Some(guard)
    }
}
/// The CombinedAllocationBacking is used by the memory manager, used for managing segments and swap, as well as for resolving demand paging and similar
pub struct CombinedAllocationBacking(Arc<CALocked>);
impl CombinedAllocationBacking {
    pub fn clone_ref(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
pub struct CombinedAllocationBackingGuard(ArcYMutexGuard<CombinedAllocationInner>);
impl_ca_lock!(CombinedAllocationBacking,CombinedAllocationBackingGuard);
// (both are wrappers around an Arc<>, they just expose different APIs)

/// VirtAllocationGuard are guards representing a mapping of a given contract/backing into virtual memory
pub struct VirtAllocationGuard {
    contract: CombinedAllocationContract,
    virt_index: usize,
}
impl core::ops::Drop for VirtAllocationGuard {
    fn drop(&mut self) {
        let mut inner = self.contract.0.lock();
        inner.clear_vmem(self.virt_index);
    }
}

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
struct AbsentPagesItemA {
    allocation: Weak<CALocked>,
    virt_allocation_index: usize,
    /// Start of this specific virtual allocation, as an offset from the "base" address of the allocation
    allocation_offset: isize,
}
struct AbsentPagesItemB {
}
type AbsentPagesTab = DescriptorTable<(),AbsentPagesItemA,AbsentPagesItemB,16,8>;
type AbsentPagesHandleA = DescriptorHandleA<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
type AbsentPagesHandleB = DescriptorHandleB<'static,(),AbsentPagesItemA,AbsentPagesItemB>;
lazy_static::lazy_static! {
    static ref ABSENT_PAGES_TABLE: AbsentPagesTab = DescriptorTable::new();
}