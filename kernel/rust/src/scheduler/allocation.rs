use crate::logging::klog;
use core::alloc::Layout;
use alloc::vec::Vec; use alloc::vec;

// TODO: move all this to part of paging? or create a new module that contains various common allocation patterns
// this isn't really "scheduler" stuff anymore
use crate::memory::physical::{PhysicalMemoryAllocation,palloc};
use crate::memory::paging::{KALLOCATION_KERNEL_STACK,PageFlags,TransitivePageFlags,MappingSpecificPageFlags,PageFrameAllocator,PageAllocation,TLPageFrameAllocator,LockedPageAllocator};
use crate::memory::paging::global_pages::{GPageFrameAllocator,KERNEL_PTABLE};

// stacks
pub struct AllocatedStack<PFA: PageFrameAllocator> {
    allocations: Vec<(PageAllocation<PFA>,Option<PhysicalMemoryAllocation>)>,
    guard_page: PageAllocation<PFA>,
    
    allocator: LockedPageAllocator<PFA>,
    flags: PageFlags,
}
impl<PFA: PageFrameAllocator> AllocatedStack<PFA> {
    fn new(phys: PhysicalMemoryAllocation, allocator: LockedPageAllocator<PFA>, virt: PageAllocation<PFA>, guard: PageAllocation<PFA>) -> Self {
        Self {
            allocations: vec![(virt,Some(phys))],
            guard_page: guard,
            allocator: allocator,
            flags: PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()),  // todo
        }
    }
    
    /* Expand the stack limit downwards by the requested number of bytes. Returns true on a success. */
    pub fn expand(&mut self, bytes: usize) -> bool {
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
                self.allocations.push((old_guard, Some(phys_og)));
                if let Some((phys_new, virt_new)) = nal { self.allocations.push((virt_new, Some(phys_new))); };
                
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

// stack
const KTASK_STACK_SIZE: usize = 256*1024;  // 256KiB TODO: figure this out better idk
const KSTACK_GUARD_SIZE: usize = 1;
pub fn allocate_ktask_stack() -> Option<AllocatedStack<GPageFrameAllocator>> {
    klog!(Debug, SCHEDULER_KTASK, "Allocating new kernel task stack.");
    let physmemallocation = palloc(Layout::from_size_align(KTASK_STACK_SIZE, 16).unwrap())?;
    let vmemallocation = KERNEL_PTABLE.allocate(physmemallocation.get_size() + KSTACK_GUARD_SIZE, KALLOCATION_KERNEL_STACK)?;
    let (guard, stack) = vmemallocation.split(KSTACK_GUARD_SIZE);  // split at the low-end as stack grows downwards
    
    guard.set_absent(0xF47B33F);
    stack.set_base_addr(physmemallocation.get_addr(), PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()));
    let bottom_addr = stack.end();
    
    klog!(Debug, SCHEDULER_KTASK, "Allocated kernel task stack at guard={:x} top={:x} bottom={:x} -> addr={:x},size={:x}.", guard.start(), stack.start(), bottom_addr, physmemallocation.get_addr(), physmemallocation.get_size());
    Some(AllocatedStack::new(physmemallocation, LockedPageAllocator::clone_ref(&KERNEL_PTABLE), stack, guard))
}