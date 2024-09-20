
use core::ops::{Deref,DerefMut};
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::sync::{Arc,Weak};
use super::paging::{LockedPageAllocator,PageFrameAllocator,AnyPageAllocation,PageAllocation,PAGE_ALIGN,PageAlignedUsize};
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
struct AllocationBacking {
    mode: AllocationBackingMode,
    size: BackingSize,
}
impl AllocationBacking {
    pub fn new(mode: AllocationBackingMode, size: BackingSize) -> Self {
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
                  combined_alloc: Weak<()>, virt_index: usize, section_identifier: usize) -> (Self,AbsentPagesHandleB) {
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
    
    /// mid MUST be < size
    pub fn split(self, mid: BackingSize, lhs_section_id: usize, rhs_section_id: usize) -> (Self,Self) {
        debug_assert!(mid < self.size());
        let Self { allocation, flags, absent_pages_table_handle: apth } = self;
        let apt_data = apth.get_a();
        let apt_alloc_weak = &apt_data.allocation;
        let virt_idx = apt_data.virt_allocation_index;
        
        let (allocation_left, allocation_right) = allocation.split_dyn(mid);
        (Self::new(allocation_left, flags, Weak::clone(apt_alloc_weak), virt_idx, lhs_section_id).0,
         Self::new(allocation_right, flags, Weak::clone(apt_alloc_weak), virt_idx, rhs_section_id).0)
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
    OffsetMapped { offset: usize },
    FixedVirtAddr { addr: usize },
}
/// Lookup an "absent page" data item in the ABSENT_PAGES_TABLE
pub fn lookup_absent_id(absent_id: usize) -> Option<((),usize)> {
    let apth_a = ABSENT_PAGES_TABLE.acquire_a(absent_id.try_into().unwrap()).ok()?;
    let apt_item_a = apth_a.get_a();
    let combined_alloc = Weak::upgrade(&apt_item_a.allocation)?;
    let virt_index = apt_item_a.virt_allocation_index;
    let section_identifier = apt_item_a.section_identifier;
    let section_obj = todo!();//AllocationSection::new(combined_alloc,section_identifier);
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

// TODO
/// Each "segment" is tied to a given allocation backing.
/// Segments have a fixed size, but may be split and (possibly merged?).
struct CombinedAllocationSegment {
    vmem: Vec<Option<VirtualAllocation>>,  // vmem is placed first in drop order so it's dropped/cleared before the backing is
    backing: AllocationBacking,
}
impl CombinedAllocationSegment {
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
        let mut lhs_vmem: Vec<Option<VirtualAllocation>> = vec_of_non_clone![None;vmem.len()];
        let mut rhs_vmem: Vec<Option<VirtualAllocation>> = vec_of_non_clone![None;vmem.len()];
        for (slot, old_alloc) in vmem.into_iter().enumerate().filter_map(|(i,o)|o.map(|x|(i,x))) {  // We only process extant vmem allocations. Nones are skipped.
            let (lhs_alloc, rhs_alloc) = old_alloc.split(mid, 0, 0);  // TODO: section identifiers?
            lhs_vmem[slot] = Some(lhs_alloc);
            rhs_vmem[slot] = Some(rhs_alloc);
        }
        
        // Done :)
        let lhs = Self { backing: lhs_backing, vmem: lhs_vmem }; lhs.remap_all_pages();
        let rhs = Self { backing: rhs_backing, vmem: rhs_vmem }; rhs.remap_all_pages();
        (lhs, rhs)
    }
    
    // VMEM MAPPING
    pub(self) fn map_vmem(&mut self, slot: usize, valloc: VirtualAllocation) {
        while slot < self.vmem.len() { self.vmem.push(None); }  // Ensure vmem has the requested slot
        
        debug_assert!(valloc.size()==self.get_size());  // ensure that the sizes match
        let prev = self.vmem[slot].replace(valloc);
        debug_assert!(prev.is_none());  // ensure we're not overwriting an in-use slot
        
        // Remap
        self.remap_pages(slot);
    }
    pub(self) fn clear_vmem(&mut self, slot: usize) {
        let old = self.vmem[slot].take();
        debug_assert!(old.is_some());  // ensure the slot was actually in use
        drop(old);  // clarity: manually drop `old` to ensure that the page table is cleared of the old mappings
    }
    
    /// Update the page table for the given virtual allocation to match any changes that have been made
    fn remap_pages(&self, slot: usize) {
        let Some(valloc) = (try{ self.vmem.get(slot)?.as_ref()? }) else { return };
        self._remap_pages_inner(valloc)
    }
    /// Update the page table for all virtual allocations, with respect to this particular segment
    fn remap_all_pages(&self) {
        for valloc in self.vmem.iter().map(Option::as_ref).flatten() {
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
struct CASegmentIndex { section: usize, segment: usize }
struct CASegmentWriter<'r> {
    inner: &'r mut CombinedAllocationInner,
    index: CASegmentIndex,
}
impl<'r> CASegmentWriter<'r> {
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
        (self_inner.get_segment_writer(self_index), Some(rhs_index))
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
        self.inner.get_segment_mut(self.index)
    }
}

/// Each "section" is a logical division. For example, working memory and the guard page would be two different sections
/// Sections may be transparently subdivided into multiple segments by the memory manager as it sees fit.
/// Sections may not be resized, but may be split/merged.
struct CombinedAllocationSection {
    segments: Vec<CombinedAllocationSegment>,
    
    /// Section identifier - a unique value assigned to each section within an allocation. This does not change until the section itself is merged/split, even if other sections are added/removed.
    section_identifier: usize,
    /// Total size in bytes
    total_size: BackingSize,
    /// Offset from the allocation "base" in vmem
    offset: isize,
}
impl CombinedAllocationSection {
    fn calculate_size(&self) -> BackingSize {
        let total = self.segments.iter().fold(0usize, |a,x| a+x.backing.size.get());
        BackingSize::new(total)
    }
    pub fn get_size(&self) -> BackingSize {
        self.total_size
    }
    pub fn get_offset(&self) -> isize {
        self.offset
    }
    pub fn get_section_identifier(&self) -> usize {
        self.section_identifier
    }
    
    // SPLITTING/MERGING
    // todo: splitting
    
    /// Merge the two sections together. Backing memory is left as-is unless overwritten afterwards.
    /// SAFETY: The caller must ensure that both sections are directly adjacent to each other, with no gaps, and with lhs occupying lower addresses than rhs.
    pub unsafe fn merge(context: &mut CombinedAllocationContext, lhs: Self, rhs: Self) -> Self {
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
        
        // Chop off a piece for each segment, one by one
        let mut remainder = allocation;
        for segment in self.segments.iter_mut().rev().skip(1).rev() {  // Skip the final segment, as we handle that below
            let (lhs,rhs) = remainder.split(segment.get_size(), 0, 0);  // TODO: Segment identifiers?
            segment.map_vmem(slot, lhs);
            remainder = rhs;
        }
        // The final segment was skipped above, because it simply takes the whole remainder
        self.segments.last_mut().unwrap().map_vmem(slot, remainder);
        
        // All done :)
    }
    pub(self) fn clear_vmem(&mut self, slot: usize) {
        for segment in self.segments.iter_mut() {
            segment.clear_vmem(slot)
        }
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct CASectionIndex(usize);
struct CASectionWriter<'r> {
    inner: &'r mut CombinedAllocationInner,
    index: CASectionIndex,
}
impl<'r> CASectionWriter<'r> {
    /// Merge two sections together. This section must be directly below the next one in memory, with no gaps.
    /// Returns Ok(merged) on success, Err(self) on failure
    pub fn merge(self, rhs: CASectionIndex) -> Result<Self,Self> {
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
        Ok(self_inner.get_section_writer(self_index))
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
        self.inner.get_section_mut(self.index)
    }
}

struct CombinedAllocationContext {
    /// The section identifier for the next section to be created
    next_section_identifier: usize,
}
impl CombinedAllocationContext {
    pub(self) fn take_next_section_identifier(&mut self) -> usize {
        let x = self.next_section_identifier;
        self.next_section_identifier += 1;
        x
    }
}
struct CombinedAllocationInner {
    sections: Vec<CombinedAllocationSection>,
    context: CombinedAllocationContext,
}
impl CombinedAllocationInner {
    fn get_section_index_from_identifier(&self, identifier: usize) -> Option<CASectionIndex> {
        Some(CASectionIndex(self.sections.iter().position(|x|x.get_section_identifier()==identifier)?))
    }
    fn get_section(&self, index: CASectionIndex) -> &CombinedAllocationSection {
        &self.sections[index.0]
    }
    fn get_section_mut(&mut self, index: CASectionIndex) -> &mut CombinedAllocationSection {
        &mut self.sections[index.0]
    }
    fn get_section_writer(&mut self, index: CASectionIndex) -> CASectionWriter {
        CASectionWriter { inner: self, index: index }
    }
    fn get_segment(&self, index: CASegmentIndex) -> &CombinedAllocationSegment {
        &self.sections[index.section].segments[index.segment]
    }
    fn get_segment_mut(&mut self, index: CASegmentIndex) -> &mut CombinedAllocationSegment {
        &mut self.sections[index.section].segments[index.segment]
    }
    fn get_segment_writer(&mut self, index: CASegmentIndex) -> CASegmentWriter {
        CASegmentWriter { inner: self, index: index }
    }
}

// TODO

use crate::descriptors::{DescriptorTable,DescriptorHandleA,DescriptorHandleB};
struct AbsentPagesItemA {
    allocation: Weak<()>,
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