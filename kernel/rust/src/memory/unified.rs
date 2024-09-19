
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
    
    /// Split this allocation into two. One of `midpoint` bytes and one of the remainder (if nonzero)
    pub fn split(self, midpoint: BackingSize) -> (Self,Option<Self>) {
        if midpoint >= self.size { return (self,None); }
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
         Some(Self::new(rhs_mode,BackingSize::new_checked(rhs_size).unwrap())))
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

// TODO

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