use crate::logging::klog;
use core::alloc::Layout;
use alloc::vec::Vec; use alloc::vec;

use crate::memory::physical::{PhysicalMemoryAllocation,palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK,PageFlags,TransitivePageFlags,MappingSpecificPageFlags,PageFrameAllocator,PageAllocation,TLPageFrameAllocator,LockedPageAllocator,PageAllocationStrategies,ALLOCATION_USER_STACK,PagingContext};
use crate::memory::paging::global_pages::{GPageFrameAllocator,KERNEL_PTABLE};

pub const MARKER_STACK_GUARD: usize = 0xF47B33F;  // "Fat Beef"
pub const MARKER_NULL_GUARD: usize = 0x4E55_4C505452;  // "NULPTR"

pub struct RealMemAllocation<PFA:PageFrameAllocator> {
    pub virt: PageAllocation<PFA>,
    pub phys: Option<PhysicalMemoryAllocation>,
}
impl<PFA:PageFrameAllocator> RealMemAllocation<PFA> {
    pub fn new(virt: PageAllocation<PFA>, phys: Option<PhysicalMemoryAllocation>) -> Self {
        Self { virt, phys }
    }
    
    pub fn allocate(allocator: &LockedPageAllocator<PFA>, size: usize, strategy: PageAllocationStrategies, flags: PageFlags) -> Option<Self> {
        let phys = palloc(Layout::from_size_align(size, 16).unwrap())?;
        let virt = allocator.allocate(phys.get_size(), strategy)?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt, phys: Some(phys) })
    }
    pub fn allocate_at(allocator: &LockedPageAllocator<PFA>, vaddr: usize, size: usize, flags: PageFlags) -> Option<Self> {
        let phys = palloc(Layout::from_size_align(size, 16).unwrap())?;
        let virt = allocator.allocate_at(vaddr, phys.get_size())?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt, phys: Some(phys) })
    }
    pub fn allocate_offset_mapped(allocator: &LockedPageAllocator<PFA>, offset: usize, size: usize, flags: PageFlags) -> Option<Self> {
        let phys = palloc(Layout::from_size_align(size, 16).unwrap())?;
        let virt = allocator.allocate_at(phys.get_addr() + offset, phys.get_size())?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt, phys: Some(phys) })
    }
}
impl<PFA: PageFrameAllocator> core::fmt::Debug for RealMemAllocation<PFA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("RealMemAllocation").field(&self.virt).field(&self.phys).finish()
    }
}

// stacks
pub struct AllocatedStack<PFA: PageFrameAllocator> {
    allocations: Vec<RealMemAllocation<PFA>>,
    guard_page: PageAllocation<PFA>,
    
    allocator: LockedPageAllocator<PFA>,
    flags: PageFlags,
}
impl<PFA: PageFrameAllocator> AllocatedStack<PFA> {
    pub fn allocate_new(allocator: &LockedPageAllocator<PFA>, size: usize, guard_size: usize, strategy: PageAllocationStrategies, flags: PageFlags) -> Option<Self> {
        klog!(Debug, MEMORY_ALLOCUTIL, "Allocating stack: size={}+{} strat={:?} flags={:?}", size, guard_size, strategy, flags);
        let physmemallocation = palloc(Layout::from_size_align(size, 16).unwrap())?;
        let vmemalloc = allocator.allocate(physmemallocation.get_size() + guard_size, strategy)?;
        let (guardalloc, stackalloc) = vmemalloc.split(guard_size);  // split at the low-end as stack grows downwards
        
        guardalloc.set_absent(MARKER_STACK_GUARD);
        stackalloc.set_base_addr(physmemallocation.get_addr(), flags);
        Some(Self {
            allocations: vec![RealMemAllocation::new(stackalloc, Some(physmemallocation))],
            guard_page: guardalloc,
            
            allocator: LockedPageAllocator::clone_ref(allocator),
            flags: flags,
        })
    }
    
    #[inline]
    /* Get the virtual address of the bottom of the stack (exclusive, so could in theory be directly assigned to RSP in x86 and grow from there with no issues). */
    pub fn bottom_vaddr(&self) -> usize {
        // Later items in allocations[] are higher in the stack (lower vmem address)
        self.allocations[0].virt.end()
    }
    
    /* Expand the stack limit downwards by the requested number of bytes. Returns true on a success. */
    pub fn expand(&mut self, bytes: usize) -> bool {
        klog!(Debug, MEMORY_ALLOCUTIL, "Expanding stack by ~{} bytes", bytes);
        let guard_size = self.guard_page.size(); let old_guard_bytes = guard_size;
        let new_alloc_bytes = bytes.saturating_sub(old_guard_bytes);
        
        let new_alloc_addr = self.guard_page.start() - new_alloc_bytes;
        let new_guard_addr = new_alloc_addr - old_guard_bytes;
        
        let new_guard = match self.allocator.allocate_at(new_guard_addr, guard_size) { Some(x)=>x, None=>return false }; new_guard.set_absent(0xF47B33F);
        let old_guard = core::mem::replace(&mut self.guard_page, new_guard);
        
        let alloc_result = 'nalloc:{
            // Allocate physical mem for upgrading old guard
            let phys_og = match palloc(Layout::from_size_align(old_guard_bytes, 16).unwrap()) { Some(x)=>x, None=>break 'nalloc None, };
            old_guard.set_base_addr(phys_og.get_addr(), self.flags);
            
            // Allocate mem for new area
            let nal = if new_alloc_bytes > 0 {Some({
                let phys_new = match palloc(Layout::from_size_align(new_alloc_bytes, 16).unwrap()) { Some(x)=>x, None=>break 'nalloc None };
                let virt_new = match self.allocator.allocate_at(new_alloc_addr, new_alloc_bytes) { Some(x)=>x, None=>break 'nalloc None };
                virt_new.set_base_addr(phys_new.get_addr(), self.flags);
                (phys_new, virt_new)
            })} else { None };
            
            Some((phys_og, nal))
        };
        match alloc_result {
            Some((phys_og, nal)) => {
                self.allocations.push(RealMemAllocation::new(old_guard, Some(phys_og)));
                if let Some((phys_new, virt_new)) = nal { self.allocations.push(RealMemAllocation::new(virt_new, Some(phys_new))); };
                
                true
            }
            None => {
                // Put the old guard back
                old_guard.set_absent(0xF47B33F);
                self.guard_page = old_guard;
                
                false
            }
        }
    }
}
impl<PFA: PageFrameAllocator> core::fmt::Debug for AllocatedStack<PFA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AllocatedStack").field("guard",&self.guard_page).field("allocations",&self.allocations).finish()
    }
}

impl AllocatedStack<GPageFrameAllocator> {
    #[inline]
    pub fn allocate_ktask() -> Option<Self> {
        Self::allocate_new(&KERNEL_PTABLE, 256*1024, 1, KALLOCATION_KERNEL_STACK, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()))
    }
}
impl AllocatedStack<TLPageFrameAllocator> {
    #[inline]
    pub fn allocate_user(context: &LockedPageAllocator<TLPageFrameAllocator>) -> Option<Self> {
        Self::allocate_new(context, 1*1024*1024, 4*4096, ALLOCATION_USER_STACK, PageFlags::new(TransitivePageFlags::USER_READABLE | TransitivePageFlags::USER_WRITEABLE, MappingSpecificPageFlags::empty()))
    }
}

pub fn new_user_paging_context() -> PagingContext {
    klog!(Debug, MEMORY_ALLOCUTIL, "Creating new user paging context.");
    let context = PagingContext::new();
    
    // null guard - 1MiB at the start to catch any null pointers
    let nullguard = context.allocate_at(0, 1*1024*1024).unwrap();
    nullguard.set_absent(MARKER_NULL_GUARD);
    nullguard.leak();  // (we'll never need to de-allocate the null guard)
    
    // :)
    context
}