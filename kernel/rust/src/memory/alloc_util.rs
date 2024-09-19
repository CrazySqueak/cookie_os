use crate::logging::klog;
use core::alloc::Layout;
use alloc::vec::Vec; use alloc::vec;
use alloc::boxed::Box;

use crate::memory::physical::{PhysicalMemoryAllocation,palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK,PageFlags,TransitivePageFlags,MappingSpecificPageFlags,PageFrameAllocator,PageAllocation,TLPageFrameAllocator,LockedPageAllocator,PageAllocationStrategies,ALLOCATION_USER_STACK,PagingContext,AnyPageAllocation,PageAlignedUsize};
use crate::memory::paging::global_pages::{GPageFrameAllocator,KERNEL_PTABLE};

pub const MARKER_STACK_GUARD: usize = 0xF47B33F;  // "Fat Beef"
pub const MARKER_NULL_GUARD: usize = 0x4E55_4C505452;  // "NULPTR"

pub struct RealMemAllocation {
    pub virt: Box<dyn AnyPageAllocation>,
    pub phys: Option<PhysicalMemoryAllocation>,
}
impl RealMemAllocation {
    pub fn new<PFA:PageFrameAllocator + Send + Sync + 'static>(virt: PageAllocation<PFA>, phys: Option<PhysicalMemoryAllocation>) -> Self {
        Self { virt: Box::new(virt), phys }
    }
    
    pub fn allocate<PFA:PageFrameAllocator + Send + Sync + 'static>(allocator: &LockedPageAllocator<PFA>, size: usize, strategy: PageAllocationStrategies, flags: PageFlags) -> Option<Self> {
        let size = PageAlignedUsize::new_rounded(size);
        let phys = palloc(size)?;
        let virt = allocator.allocate(phys.get_size(), strategy)?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt: Box::new(virt), phys: Some(phys) })
    }
    pub fn allocate_at<PFA:PageFrameAllocator + Send + Sync + 'static>(allocator: &LockedPageAllocator<PFA>, vaddr: usize, size: usize, flags: PageFlags) -> Option<Self> {
        let size = PageAlignedUsize::new_rounded(size);
        let phys = palloc(size)?;
        let virt = allocator.allocate_at(vaddr, phys.get_size())?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt: Box::new(virt), phys: Some(phys) })
    }
    pub fn allocate_offset_mapped<PFA:PageFrameAllocator + Send + Sync + 'static>(allocator: &LockedPageAllocator<PFA>, offset: usize, size: usize, flags: PageFlags) -> Option<Self> {
        let size = PageAlignedUsize::new_rounded(size);
        let phys = palloc(size)?;
        let virt = allocator.allocate_at(phys.get_addr() + offset, phys.get_size())?;
        virt.set_base_addr(phys.get_addr(), flags);
        Some(Self { virt: Box::new(virt), phys: Some(phys) })
    }
}
impl core::fmt::Debug for RealMemAllocation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("RealMemAllocation").field(&self.virt).field(&self.phys).finish()
    }
}

// stacks
pub struct AllocatedStack<PFA: PageFrameAllocator> {
    allocations: Vec<RealMemAllocation>,
    guard_page: PageAllocation<PFA>,
    
    allocator: LockedPageAllocator<PFA>,
    flags: PageFlags,
}
impl<PFA: PageFrameAllocator + Send + Sync + 'static> AllocatedStack<PFA> {
    pub fn allocate_new(allocator: &LockedPageAllocator<PFA>, size: PageAlignedUsize, guard_size: PageAlignedUsize, strategy: PageAllocationStrategies, flags: PageFlags) -> Option<Self> {
        // klog!(Debug, MEMORY_ALLOCUTIL, "Allocating stack: size={}+{} strat={:?} flags={:?}", size, guard_size, strategy, flags);
        // let physmemallocation = palloc(size)?;
        // let vmemalloc = allocator.allocate(physmemallocation.get_size().get() + guard_size.get(), strategy)?;
        todo!() // Some(Self::from_allocations(allocator, guard_size, physmemallocation, vmemalloc, flags))
    }
    fn from_allocations(allocator: &LockedPageAllocator<PFA>, guard_size: usize, physmemallocation: PhysicalMemoryAllocation, vmemalloc: PageAllocation<PFA>, flags: PageFlags) -> Self {
        // let (guardalloc, stackalloc) = vmemalloc.split(guard_size);  // split at the low-end as stack grows downwards
        // 
        // guardalloc.set_absent(MARKER_STACK_GUARD);
        // stackalloc.set_base_addr(physmemallocation.get_addr(), flags);
        // Self {
        //     allocations: vec![RealMemAllocation::new(stackalloc, Some(physmemallocation))],
        //     guard_page: guardalloc,
        //     
        //     allocator: LockedPageAllocator::clone_ref(allocator),
        //     flags: flags,
        todo!() // }
    }
    
    #[inline]
    /* Get the virtual address of the bottom of the stack (exclusive, so could in theory be directly assigned to RSP in x86 and grow from there with no issues). */
    pub fn bottom_vaddr(&self) -> usize {
        // Later items in allocations[] are higher in the stack (lower vmem address)
        self.allocations[0].virt.end()
    }
    
    /* Expand the stack limit downwards by the requested number of bytes. Returns true on a success. */
    pub fn expand(&mut self, bytes: usize) -> bool {
        // klog!(Debug, MEMORY_ALLOCUTIL, "Expanding stack by ~{} bytes", bytes);
        // let guard_size = self.guard_page.size(); let old_guard_bytes = guard_size;
        // let new_alloc_bytes = bytes.saturating_sub(old_guard_bytes);
        // 
        // let new_alloc_addr = self.guard_page.start() - new_alloc_bytes;
        // let new_guard_addr = new_alloc_addr - old_guard_bytes;
        // 
        // let new_guard = match self.allocator.allocate_at(new_guard_addr, guard_size) { Some(x)=>x, None=>return false }; new_guard.set_absent(0xF47B33F);
        // let old_guard = core::mem::replace(&mut self.guard_page, new_guard);
        // 
        // let alloc_result = 'nalloc:{
        //     // Allocate physical mem for upgrading old guard
        //     let phys_og = match palloc(Layout::from_size_align(old_guard_bytes, 16).unwrap()) { Some(x)=>x, None=>break 'nalloc None, };
        //     old_guard.set_base_addr(phys_og.get_addr(), self.flags);
        //     
        //     // Allocate mem for new area
        //     let nal = if new_alloc_bytes > 0 {Some({
        //         let phys_new = match palloc(Layout::from_size_align(new_alloc_bytes, 16).unwrap()) { Some(x)=>x, None=>break 'nalloc None };
        //         let virt_new = match self.allocator.allocate_at(new_alloc_addr, new_alloc_bytes) { Some(x)=>x, None=>break 'nalloc None };
        //         virt_new.set_base_addr(phys_new.get_addr(), self.flags);
        //         (phys_new, virt_new)
        //     })} else { None };
        //     
        //     Some((phys_og, nal))
        // };
        // match alloc_result {
        //     Some((phys_og, nal)) => {
        //         self.allocations.push(RealMemAllocation::new(old_guard, Some(phys_og)));
        //         if let Some((phys_new, virt_new)) = nal { self.allocations.push(RealMemAllocation::new(virt_new, Some(phys_new))); };
        //         
        //         true
        //     }
        //     None => {
        //         // Put the old guard back
        //         old_guard.set_absent(0xF47B33F);
        //         self.guard_page = old_guard;
        //         
        //         false
        //     }
        todo!() // }
    }
}
impl<PFA: PageFrameAllocator> core::fmt::Debug for AllocatedStack<PFA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AllocatedStack").field("guard",&self.guard_page).field("allocations",&self.allocations).finish()
    }
}

pub type GAllocatedStack = AllocatedStack<GPageFrameAllocator>;
impl GAllocatedStack {
    #[inline]
    pub fn allocate_ktask() -> Option<Self> {
        Self::allocate_new(&KERNEL_PTABLE, PageAlignedUsize::new_rounded(256*1024), PageAlignedUsize::new_rounded(1), KALLOCATION_KERNEL_STACK, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()))
    }
    /* Allocates a stack for a newly starting CPU. This stack is placed as early in memory as possible to guarantee it is mapped in the bootstrap page table as well.
        Additionally, it is offset-mapped rather than */
    #[inline]
    pub fn allocate_kboot() -> Option<Self> {
        // TBH i don't particularly remember how this works because brain fog but it works now so yay
        let alloc_size = 256*1024; let guard_size = 4096;
        // Note: we keep retrying until we get one that's able to be offset mapped
        // (slow but eh it'll hold)
        let mut failed = alloc::vec::Vec::<PhysicalMemoryAllocation>::new();
        let (physalloc, vmemalloc) = loop {
            let physalloc = palloc(PageAlignedUsize::new_rounded(alloc_size))?;
              // make sure the guard page factored in despite not occupying any real memory
            match KERNEL_PTABLE.allocate_at(KERNEL_PTABLE.get_vmem_offset()+physalloc.get_addr()-guard_size, PageAlignedUsize::new_rounded(physalloc.get_size().get()+guard_size)){
                Some(vmemalloc) => break (physalloc, vmemalloc),  // OK :)
                None => {
                    // We failed to offset-map that section of memory, so try again.
                    failed.push(physalloc);  // store the failed allocation in a vec for now, so that the allocator is forced to give us a new chunk of memory (but the unused physical allocations are dropped again once we're finished)
                    continue;
                }
            }
        };
        Some(Self::from_allocations(&KERNEL_PTABLE, guard_size, physalloc, vmemalloc, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty())))
    }
}
pub type TLAllocatedStack = AllocatedStack<TLPageFrameAllocator>;
impl TLAllocatedStack {
    #[inline]
    pub fn allocate_user(context: &LockedPageAllocator<TLPageFrameAllocator>) -> Option<Self> {
        Self::allocate_new(context, PageAlignedUsize::new_rounded(1*1024*1024), PageAlignedUsize::new_rounded(4*4096), ALLOCATION_USER_STACK, PageFlags::new(TransitivePageFlags::USER_READABLE | TransitivePageFlags::USER_WRITEABLE, MappingSpecificPageFlags::empty()))
    }
}

/* Any allocated stack, regardless of PFA */
pub trait AnyAllocatedStack: core::fmt::Debug + Send {
    fn bottom_vaddr(&self) -> usize;
    fn expand(&mut self, bytes: usize) -> bool;
}
impl<PFA: PageFrameAllocator + Send + Sync + 'static> AnyAllocatedStack for AllocatedStack<PFA> {
    fn bottom_vaddr(&self) -> usize { self.bottom_vaddr() }
    fn expand(&mut self, bytes: usize) -> bool { self.expand(bytes) }
}

pub fn new_user_paging_context() -> PagingContext {
    klog!(Debug, MEMORY_ALLOCUTIL, "Creating new user paging context.");
    let context = PagingContext::new();
    
    // null guard - 1MiB at the start to catch any null pointers
    let nullguard = context.allocate_at(0, PageAlignedUsize::new_rounded(1*1024*1024)).unwrap();
    nullguard.set_absent(MARKER_NULL_GUARD);
    nullguard.leak();  // (we'll never need to de-allocate the null guard)
    
    // :)
    context
}