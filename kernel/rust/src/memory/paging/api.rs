
use core::sync::atomic::{AtomicU16,Ordering};
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::sync::{YRwLock as RwLock,YRwLockReadGuard as RwLockReadGuard,YRwLockWriteGuard as RwLockWriteGuard,YRwLockUpgradableGuard as RwLockUpgradableGuard};  // changed to using YLocks for now as Wlocks aren't re-implemented yet
use crate::multitasking::cpulocal::CpuLocal;

use super::*;

type BaseTLPageAllocator = arch::TopLevelPageAllocator;
use arch::{set_active_page_table,inval_tlb_pg};

// FLAGS & STUFF
#[derive(Debug,Clone,Copy)]
pub struct PageFlags {
    pub tflags: TransitivePageFlags,
    pub mflags: MappingSpecificPageFlags,
}
impl PageFlags {
    pub fn new(tflags: TransitivePageFlags, mflags: MappingSpecificPageFlags) -> Self {
        Self { tflags, mflags }
    }
    pub fn empty() -> Self {
        Self::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty())
    }
}
impl core::ops::BitAnd<TransitivePageFlags> for PageFlags {
    type Output = Self;
    fn bitand(self, rhs: TransitivePageFlags) -> Self::Output {
        Self::new(self.tflags & rhs, self.mflags)
    }
}
impl core::ops::BitOr<TransitivePageFlags> for PageFlags {
    type Output = Self;
    fn bitor(self, rhs: TransitivePageFlags) -> Self::Output {
        Self::new(self.tflags | rhs, self.mflags)
    }
}
impl core::ops::BitAnd<MappingSpecificPageFlags> for PageFlags {
    type Output = Self;
    fn bitand(self, rhs: MappingSpecificPageFlags) -> Self::Output {
        Self::new(self.tflags, self.mflags & rhs)
    }
}
impl core::ops::BitOr<MappingSpecificPageFlags> for PageFlags {
    type Output = Self;
    fn bitor(self, rhs: MappingSpecificPageFlags) -> Self::Output {
        Self::new(self.tflags, self.mflags | rhs)
    }
}
impl core::ops::BitAnd<Self> for PageFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self::new(self.tflags & rhs.tflags, self.mflags & rhs.mflags)
    }
}
impl core::ops::BitOr<Self> for PageFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self::new(self.tflags | rhs.tflags, self.mflags | rhs.mflags)
    }
}
bitflags::bitflags! {
    // These flags follow a "union" pattern - flags applied to upper levels will also override lower levels (the most restrictive version winning)
    // Therefore: the combination of all flags should be the most permissive/compatible option
    #[derive(Debug,Clone,Copy)]
    pub struct TransitivePageFlags: u16 {
        // User can access this page.
        const USER_READABLE = 1<<0;
        // User can write to this page (provided they have access to it).
        const USER_WRITEABLE = 1<<1;
        // Execution is allowed. (if feature per_page_NXE_bit is not enabled then this is ignored)
        const EXECUTABLE = 1<<2;
    }
    // These flags are non-transitive. For regular mappings, they work the same as transitive flags. For sub-table mappings, they apply to the sub-table itself, rather than any descendant page mappings.
    #[derive(Debug,Clone,Copy)]
    pub struct MappingSpecificPageFlags: u16 {
        // Page mapping is global (available in all address spaces), and should not be flushed from the TLB on an address-space switch.
        // On pages: Page is not flushed from the TLB on address-space switch.
        // On sub-tables: Architecture dependent.
        const GLOBAL = 1<<0;
        // Page is pinned. Pinned pages must not be moved in (or swapped out of) physical memory, as they may contain important structures such as page tables or interrupt handlers.
        // This is a custom flag which is interpreted by the OS' page handler
        // On pages: Page must always point to the same frame. Frame's contents must not be moved.
        // On sub-tables: No effect.
        const PINNED = 1<<1;
        // Affects the "memory type" of the selected area - idk what this does because what I've read is very vague
        // On pages: influences the memory type of the memory mapped via the page
        // On sub-tables: influences the memory type used for reading the sub-table
        const CACHE_DISABLE = 1<<2;
        // Affects the "memory type" of the selected area - enables "write-through caching", meaning that writes occur immediately instead of being cached and then written back later
        // On pages: influences the memory type of the memory mapped via the page
        // On sub-tables: influences the memory type used for reading the sub-table
        const CACHE_WRITE_THROUGH = 1<<3;
    }
}

macro_rules! pageFlags {
    ($($k:ident:$i:ident),*) => {{
        use $crate::memory::paging::{PageFlags,TransitivePageFlags as t,MappingSpecificPageFlags as m};
        PageFlags::empty() $(| $k::$i)+
    }}
}
pub(crate) use pageFlags;

// LOCKED PAGE ALLOCATOR
#[derive(Debug,Clone,Copy)]
pub struct LPAMetadata {
    // Offset in VMEM for the start of the page table's jurisdiction. For top-level tables this is 0. For tables nested inside of other tables, this might not be 0
    pub offset: usize,
    // Default options for new write guards
    pub default_options: LPAWGOptions,
}
struct LPAInternal<PFA: PageFrameAllocator> {
    // The locked allocator
    lock: RwLock<PFA>,
    // Other metadata
    meta: LPAMetadata,
    // The number of times this is "active" - usually 1 for global tables, or <number of CPUs table is active for> for top-level contexts.
    active_count: AtomicU16,
    // The IDs of the CPUs this is active on, used for flushing the TLB for the correct CPUs (if non-global. For global pages this is usually just the ID of the bootstrap processor and nothing else :-/)
    // Note that if you just need to know the number of CPUs, active_count is likely to be more accurate (as it's intended for that rather than this which is just for the TLB invalidation process)
    active_on: RwLock<Vec<usize>>,
}
impl<PFA: PageFrameAllocator> LPAInternal<PFA> {
    fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self {
            lock: RwLock::new(alloc),
            meta: meta,
            active_count: AtomicU16::new(0),
            active_on: RwLock::new(Vec::new()),
        }
    }
    pub fn metadata(&self) -> LPAMetadata {
        self.meta
    }
}
impl<PFA: PageFrameAllocator> core::ops::Deref for LPAInternal<PFA>{
    type Target = RwLock<PFA>;
    fn deref(&self) -> &Self::Target {
        &self.lock
    }
}

pub struct LockedPageAllocator<PFA: PageFrameAllocator>(Arc<LPAInternal<PFA>>);
impl<PFA: PageFrameAllocator> LockedPageAllocator<PFA> {
    pub fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self(Arc::new(LPAInternal::new(alloc, meta)))
    }
    
    pub fn clone_ref(x: &Self) -> Self {
        Self(Arc::clone(&x.0))
    }
    
    pub fn metadata(&self) -> &LPAMetadata {
        &(*self.0).meta
    }
    
    /* Lock the allocator for reading until _end_active is called.
    This is intended to be used when the page table is possibly being read/cached by the CPU, as when locked by _begin_active, writes via write_when_active are still possible (as they flush the TLB). */
    pub(super) fn _begin_active(&self) -> &PFA {
        // Lock (by obtaining then forgetting a read guard, and using the underlying ptr instead)
        core::mem::forget(self.0.read());
        let alloc = unsafe{ &*self.0.data_ptr() };  // SAFETY: We hold a read lock which we obtained (and then leaked) in the statement above
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        // Increment active count
        self.0.active_count.fetch_add(1, Ordering::Acquire);
        // Add self to active_on
        self.0.active_on.write().push(crate::multitasking::get_cpu_num());
        // Return
        alloc
    }
    pub(super) unsafe fn _end_active(&self){
        // Remove from active_on
        let mut wg = self.0.active_on.write();
        let cpu_num = crate::multitasking::get_cpu_num();
        if let Some(pos) = wg.iter().position(|x|*x==cpu_num) {
            wg.swap_remove(pos);
        }
        drop(wg);
        // Decrement active count
        self.0.active_count.fetch_sub(1, Ordering::Release);
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        // Unlock
        self.0.force_unlock_read();  // this should really be called force_read_decrement tbh. would be much less ambiguous
    }
    
    pub(super) fn read(&self) -> RwLockReadGuard<PFA> {
        self.0.read()
    }
    
    pub(super) fn write(&self) -> LPageAllocatorRWLWriteGuard<PFA> {
        let options = self.metadata().default_options;
        
        LockedPageAllocatorWriteGuard{guard: self.0.write(), allocator: Self::clone_ref(self), options}
    }
    pub(super) fn try_write(&self) -> Option<LPageAllocatorRWLWriteGuard<PFA>> {
        match self.0.try_write() {
            Some(guard) => {
                let options = self.metadata().default_options;
                
                Some(LockedPageAllocatorWriteGuard{guard, allocator: Self::clone_ref(self), options})
            },
            None => None,
        }
    }
    
    /* Write to a page table that is currently active, provided there are no other read/write locks.
        Writes using this guard will automatically invalidate the TLB entries as needed.*/
    pub(super) fn write_when_active(&self) -> LPageAllocatorUnsafeWriteGuard<PFA> {
        // Acquire an upgradable read guard, to ensure that A. there are no writers, and B. there are no new readers while we're determining what to do
        let upgradable = self.0.upgradable_read();
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        
        // Ensure reader count matches our active_count
        /* SAFETY / SYNC:
           Since no new readers can be allocated, reader_count will not have increased during this time
           Since reader_count is incremented before active_count, if a race occurs during _begin_active, then reader_count > active_count, which means a lock is not acquired and we must wait and try again.
           Similarly, active_count must be decremented before reader_count in _end_active. If a race occurs during _end_active, then reader_count > active_count, which means a lock is not acquired blah blah blah
           We also read reader_count before active_count, to ensure that in a race with _end_active, we will end up with reader_count > active_count (since active_count is decremented first).
           
           Note: Reader count no longer includes the upgradeable guard (however since we have an upgradeable guard, this returns Err(x))
        */
        loop {
            // Safety: .raw() shouldn't be unsafe due to allowing you to unlock it as the unlock...() functions are already unsafe! (as is poking at implementation details)
            let reader_count = unsafe{self.0.raw()}.reader_count().err().unwrap();
            core::sync::atomic::compiler_fence(Ordering::SeqCst);
            let active_count = self.0.active_count.load(Ordering::Acquire);
            if reader_count <= (active_count+1).into() {
                // Force-upgrade and return the write lock
                // We cannot upgrade normally because that would require there to be no readers
                // Whereas here we've ensured that the readers are all CPU/TLB readers or something
                // So we want to forcibly create an upgraded version
                let write_guard = unsafe { ForcedUpgradeGuard::new(&self.0, upgradable) };
                
                let mut options = self.metadata().default_options;
                options.auto_flush_tlb = active_count > 0;  // if we are active in any contexts, we should remember to flush TLB
                
                return LockedPageAllocatorWriteGuard{guard: write_guard, allocator: Self::clone_ref(self), options};
            } else {
                // Relax
                // 🛏☺☕ ahhh
                crate::multitasking::scheduler::spin_yield();
            }
        }
    }
    
    pub fn allocate(&self, size: usize, alloc_strat: PageAllocationStrategies) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate(size, alloc_strat)
    }
    pub fn allocate_at(&self, addr: usize, size: usize) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate_at(addr, size)
    }
    /// Allocate page(s) dynamically such that the given physical address would be able to be mapped using a simple .set_addr (i.e. such that phys minus base would be page-aligned)
    pub fn allocate_alignedoffset(&self, size: usize, alloc_strat: PageAllocationStrategies, phys_addr: usize) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate_alignedoffset(size, alloc_strat, phys_addr)
    }
}

pub struct PagingContext(LockedPageAllocator<BaseTLPageAllocator>);
impl PagingContext {
    pub fn new() -> Self {
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Creating new paging context.");
        let mut allocator = BaseTLPageAllocator::new();
        // Add global pages
        for i in 0..global_pages::N_GLOBAL_TABLES {
            let phys_addr = global_pages::GLOBAL_TABLE_PHYSADDRS[i];
            let flags = global_pages::GLOBAL_TABLE_FLAGS[i];
            // SAFETY: See documentation for put_global_table and GLOBAL_TABLE_PHYSADDRS
            unsafe{ allocator.put_global_table(global_pages::GLOBAL_PAGES_START_IDX+i, phys_addr, flags); }
        }
        // Return
        let dopts = LPAWGOptions::new_default();
        Self(LockedPageAllocator::new(allocator, LPAMetadata { offset: 0, default_options: dopts }))
    }
    pub fn clone_ref(x: &Self) -> Self {
        Self(LockedPageAllocator::clone_ref(&x.0))
    }
    
    /* Activate this page table. Once active, this page table will be used to map virtual addresses to physical ones.
        Use of Arc ensures that the page table will not be dropped if it is still active.
        DEADLOCK: If you have a write guard active in the current thread, this *WILL* deadlock.
        DANGER: You MUST follow the proper rules unless you want your program to CRASH UNEXPECTEDLY! This is serious shit!
         * The kernel stack should be at the same virtual memory address in both the old and new tables. It cannot simply be moved to a different one.
         * All heap objects and objects pointed to by pointers should be at the same virtual memory address in both the old and new tables. Any pointers or objects that are not at the same VMem address will cause Undefined Behaviour if accessed (unless the old page table is restored).
         * All kernel code you plan to call must be at the same addresses in both the old and new tables. Most important are INTERRUPT HANDLERS and the PANIC HANDLER (as well as common utilities such as klog). This also includes the activate() function and the function you called it from. (and the static variable that stores the page table)
         The easiest way to achieve the above three points is to map the kernel to the same position in every page table. This is why the kernel lives in the higher half - it should never be necessary to change its location in virtual memory.
         */
    pub unsafe fn activate(&self){
        // Leak read guard (as the TLB will cache the page table as needed, thus meaning it should not be modified without careful consideration)
        let allocator = self.0._begin_active();
        
        // activate table
        let table_addr = ptaddr_virt_to_phys(allocator.get_page_table_ptr() as usize);
        klog!(Info, MEMORY_PAGING_CONTEXT, "Switching active context to 0x{:x}", table_addr);
        
        let ni = disable_interruptions();
        // Set active
        set_active_page_table(table_addr);
        // store reference (and take old one)
        let oldpt = _ACTIVE_PAGE_TABLE.lock().replace(Self::clone_ref(&self));
        // Enable interruptions
        drop(ni);
        
        // Decrement reader count on old page table (if applicable)
        // Safety: Since the previous page table was activated using this function,
        //         which leaks a read guard, we can be sure that decrementing the
        //         counter here will be defined, working as if the guard had been dropped.
        // (N.B. we can't simply store the guard due to borrow checker limitations + programmer laziness)
        if let Some(old_table) = oldpt { unsafe {
            old_table.0._end_active();
        }}
    }
}
impl core::ops::Deref for PagingContext {
    type Target = LockedPageAllocator<BaseTLPageAllocator>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// = GUARDS =
#[derive(Debug,Clone,Copy)]
pub struct LPAWGOptions {
    pub(super) auto_flush_tlb: bool,
    pub(super) is_global_page: bool,
}
impl LPAWGOptions {
    pub(super) fn new_default() -> Self {
        Self {
            auto_flush_tlb: false,
            is_global_page: false,
        }
    }
}

pub struct LockedPageAllocatorWriteGuard<PFA: PageFrameAllocator, GuardT>
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA> 
  {
    guard: GuardT, allocator: LockedPageAllocator<PFA>,
    
    pub(super) options: LPAWGOptions,
}
//pub type TLPageAllocatorWriteGuard<'a> = LockedPageAllocatorWriteGuard<'a, BaseTLPageAllocator>;
pub type LPageAllocatorRWLWriteGuard<'a, PFA> = LockedPageAllocatorWriteGuard<PFA, RwLockWriteGuard<'a, PFA>>;
pub type LPageAllocatorUnsafeWriteGuard<'a, PFA> = LockedPageAllocatorWriteGuard<PFA, ForcedUpgradeGuard<'a, PFA>>;

macro_rules! ppa_define_foreach {
    // Note: body takes four variables from the outside: allocator, ptable, index, and offset (as well as any other args they specify).
    ($($fkw:ident)*: $fnname: ident, $pfaname:ident:&mut PFA, $allocname:ident:&PPA, $ptname:ident:&mut IPT, $idxname:ident:usize, $offname:ident:usize, $($argname:ident:$argtype:ty),*, $body:block, $sabody: block) => {
        $($fkw)* fn $fnname($pfaname: &mut impl PageFrameAllocator, $allocname: &PartialPageAllocation, parentoffset: usize, $($argname:$argtype),*){
            let $ptname = $pfaname.get_page_table_mut();
            // Call on current-level entries
            for item in $allocname.entries() {
                match item {
                    &PAllocItem::Page { index: $idxname, offset } => {
                        let $offname = parentoffset+offset;
                        $body
                    },
                    &PAllocItem::SubTable { index: $idxname, offset, .. } => {
                        let $offname = parentoffset+offset;
                        $sabody
                    },
                }
            }
            // Recurse into sub-allocators
            for item in $allocname.entries() {
                if let &PAllocItem::SubTable { index, offset, alloc: ref suballocation } = item {
                    let suballocator = $pfaname.get_suballocator_mut(index).expect("Allocation expected sub-allocator but none was found!");
                    Self::$fnname(suballocator,suballocation, parentoffset+offset, $($argname),*);
                }
            }
        }
    }
}
impl<PFA: PageFrameAllocator, GuardT> LockedPageAllocatorWriteGuard<PFA, GuardT> 
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA>
  {
    fn meta(&self) -> &LPAMetadata {
        self.allocator.metadata()
    }
    fn get_page_table(&mut self) -> &mut PFA {
        &mut*self.guard
    }
    /* Used for logging */
    #[inline(always)]
    fn _pt_phys_addr(&mut self) -> usize {
        ptaddr_virt_to_phys(self.get_page_table().get_page_table_ptr() as usize)
    }
    #[inline(always)]
    fn _fmt_pa(&mut self, allocation: &PageAllocation<PFA>) -> alloc::string::String {
        alloc::format!("{:x}[{:?}]", self._pt_phys_addr(), allocation.allocation)
    }
    
    // Allocating
    // (we don't need to flush the TLB for allocation as the page has gone from NOT PRESENT -> NOT PRESENT - instead we flush it when it's mapped to an address)
    pub(super) fn allocate(&mut self, size: usize, alloc_strat: PageAllocationStrategies) -> Option<PageAllocation<PFA>> {
        let allocator = self.get_page_table();
        let allocation = allocator.allocate(size, alloc_strat)?;
        Some(PageAllocation::new(LockedPageAllocator::clone_ref(&self.allocator), allocation))
    }
    pub(super) fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PageAllocation<PFA>> {
        // Convert the address
        // Subtract the offset from the vmem address (as we need it to be relative to the start of our table)
        let rel_addr = addr.checked_sub(self.meta().offset).unwrap_or_else(||panic!("Cannot allocate memory before the start of the page! addr=0x{:x} page_start=0x{:x}",addr,self.meta().offset));
        
        // Allocate
        let allocator = self.get_page_table();
        let allocation = allocator.allocate_at(rel_addr, size)?;
        let mut palloc = PageAllocation::new(LockedPageAllocator::clone_ref(&self.allocator), allocation);
        palloc.baseaddr_offset = addr - palloc.start();  // Figure out the offset between the requested address and the actual start of the allocation, if required
        Some(palloc)
    }
    /// Allocate page(s) dynamically such that the given physical address would be able to be mapped using a simple .set_addr (i.e. such that phys minus base would be page-aligned)
    pub(super) fn allocate_alignedoffset(&mut self, size: usize, alloc_strat: PageAllocationStrategies, phys_addr: usize) -> Option<PageAllocation<PFA>> {
        // Step 1: round down the physical address to page alignment
        use super::MIN_PAGE_SIZE;
        let (_, allocation_offset) = (phys_addr / MIN_PAGE_SIZE, phys_addr % MIN_PAGE_SIZE);
        // Step 2: increase size by the remainder to compensate
        let allocated_size = size + allocation_offset;
        // Step 3: Allocate
        let mut allocation = self.allocate(allocated_size, KALLOCATION_DYN_MMIO)?;
        allocation.baseaddr_offset += allocation_offset;
        Some(allocation)
    }
    
    /* Split the allocation into two separate allocations. The first one will contain bytes [0,n) (rounding up if not page aligned), and the second one will contain the rest.
       If necessary, huge pages will be split to meet the midpoint as accurately as possible.
       Note: It is not guaranteed that the second allocation will not be empty. */
    pub(super) fn split_allocation(&mut self, allocation: PageAllocation<PFA>, mid: usize) -> (PageAllocation<PFA>, PageAllocation<PFA>) {
        allocation.assert_pt_tag(self);
        let baseaddr_offset = allocation.baseaddr_offset;
        let (allocation, allocator, metadata) = allocation.leak();
        
        let (lhs, rhs) = Self::_split_alloc_inner(self.get_page_table(), allocation, mid);
        (
            PageAllocation { allocator: LockedPageAllocator::clone_ref(&allocator), allocation: lhs, metadata, baseaddr_offset },
            PageAllocation { allocator:                                 allocator , allocation: rhs, metadata, baseaddr_offset },
        )
    }
    fn _split_alloc_inner<SPF:PageFrameAllocator>(pfa: &mut SPF, allocation: PartialPageAllocation, mid: usize) -> (PartialPageAllocation, PartialPageAllocation) {
        use alloc::collections::vec_deque::VecDeque;
        let mut lhs = Vec::<PAllocItem>::new();
        let mut rhs = Vec::<PAllocItem>::new();
        
        let mut entries = VecDeque::from(allocation.into_entries());
        
        while let Some(item) = entries.pop_front() {
            if item.offset() < mid {
                // LHS or "pivot"
                if entries.front().map_or(true, |ni| ni.offset() > mid) {
                    // Next item is after the mid-point, so this item is the pivot (has the midpoint INSIDE it rather than on a boundrary)
                    // (if we're at the end and not past the midpoint, then the final item is considered the pivot)
                    match item {
                        PAllocItem::SubTable { index, offset, alloc: suballoc } => {
                            // It's a table, so we can split this if recurse
                            let suballocator = pfa.get_suballocator_mut(index).expect("Allocation expected sub-allocator but none was found!");
                            let (left, right) = Self::_split_alloc_inner(suballocator, suballoc, mid.checked_sub(offset).unwrap());
                            lhs.push(PAllocItem::SubTable { index, offset, alloc: left });
                            rhs.push(PAllocItem::SubTable { index, offset, alloc: right });
                        }
                        
                        PAllocItem::Page { index, offset } => {
                            // Attempt to split the page
                            let result = pfa.split_page(index);
                            if let Ok(suballocation) = result {
                                // Success - split the newly created table
                                let suballocator = pfa.get_suballocator_mut(index).unwrap();
                                let (left, right) = Self::_split_alloc_inner(suballocator, suballocation, mid.checked_sub(offset).unwrap());
                                lhs.push(PAllocItem::SubTable { index, offset, alloc: left });
                                rhs.push(PAllocItem::SubTable { index, offset, alloc: right });
                            } else {
                                // \_(o.o)_/
                                // round up so lhs.size is always >= mid
                                lhs.push(PAllocItem::Page { index, offset });
                            }
                        }
                    }
                } else {
                    // LHS
                    lhs.push(item);
                }
            } else {
                // RHS
                rhs.push(item);
            }
        }
        
        // Adjust RHS offsets so they are based on the RHS rather than the original allocation as a whole
        // (it's sorted (as this algo is stable and sorting is required) so we just take the first one lmao)
        if !rhs.is_empty() {
            let min_offset = rhs[0].offset();
            for item in &mut rhs {
                *item.offset_mut() -= min_offset;
            }
        }
        
        // Return allocations
        (
            PartialPageAllocation::new(lhs, SPF::PAGE_SIZE),
            PartialPageAllocation::new(rhs, SPF::PAGE_SIZE),
        )
    }
    
    /* drop() only gives us a reference so we have to make do */
    pub(self) fn dealloc(&mut self, allocation: &PageAllocation<PFA>){
        allocation.assert_pt_tag(self);
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Deallocating {}", self._fmt_pa(allocation));
        self.get_page_table().deallocate(&allocation.allocation);
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    
    // Managing allocations
    /* Set the base physical address for the given allocation. This also sets the PRESENT flag automatically. */
    pub(super) fn set_base_addr(&mut self, allocation: &PageAllocation<PFA>, base_addr: usize, mut flags: PageFlags){
        allocation.assert_pt_tag(self);
        
        if self.options.is_global_page { flags.mflags |= MappingSpecificPageFlags::GLOBAL; }
        
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Mapping {} to base addr {:x} (flags={:?})", self._fmt_pa(allocation), base_addr, flags);
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        unsafe {
            Self::_set_addr_inner(self.get_page_table(), allocation.into(), 0, base_addr, flags);
        }
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    ppa_define_foreach!(unsafe: _set_addr_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, base_addr: usize, flags: PageFlags, {
        ptable.set_huge_addr(index, base_addr+offset, flags);
    }, { ptable.add_subtable_flags::<false>(index, &flags); });
    
    /* Set the given allocation as absent (not in physical memory). */
    pub(super) fn set_absent(&mut self, allocation: &PageAllocation<PFA>, data: usize){
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        allocation.assert_pt_tag(self);
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Mapping {} as absent (data={:x})", self._fmt_pa(allocation), data);
        unsafe {
            Self::_set_missing_inner(self.get_page_table(), allocation.into(), 0, data);
        }
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    ppa_define_foreach!(unsafe: _set_missing_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, data: usize, {
        ptable.set_absent(index, data);
    }, {});
    
    /* Invalidate the TLB entries for the given allocation (on the current CPU).
        Note: No check is performed to ensure that the allocation is correct nor that this page table is active, as the only consequence (provided all other code handling Page Tables / TLB is correct) is a performance hit from the unnecessary INVLPG operations + the resulting cache misses.
        Note: Using this method is unnecessary yourself. Usually it is provided by write_when_active or similar. */
    pub(super) fn invalidate_tlb(&mut self, allocation: &PageAllocation<PFA>){
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Flushing TLB for {:?}", allocation.allocation);
        let vmem_offset = allocation.start();  // (vmem offset is now added by PageAllocation itself)
        let active_on = self.allocator.0.active_on.read(); // It's faster to hold the lock here than to clone as read/write contention on the same page allocator is negligble compared to contention on the heap allocator
        inval_tlb_pg(allocation.into(), allocation.metadata.offset, self.options.is_global_page, Some(&*active_on));
    }
}

pub struct ForcedUpgradeGuard<'a, T> {
    guard: RwLockUpgradableGuard<'a, T>,
    ptr: &'a mut T,
}
impl<'a,T> ForcedUpgradeGuard<'a, T>{
    // SAFETY: HERE BE DRAGONS
    unsafe fn new(lock: &RwLock<T>, guard: RwLockUpgradableGuard<'a, T>) -> Self {
        use core::ops::Deref;
        // 👻👻👻👻👻
        // TODO: Find a better alternative
        // This dirty hack gives me the creeps
        let ptr = &mut *lock.data_ptr();
        Self {
            guard,
            ptr,
        }
    }
}
impl<T> core::ops::Deref for ForcedUpgradeGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.guard
    }
}
impl<T> core::ops::DerefMut for ForcedUpgradeGuard<'_, T>{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ptr
    }
}

// = ACTIVE OR SMTH? =
// the currently active page table on each CPU
use crate::sync::kspin::KMutex;
use crate::multitasking::disable_interruptions;
static _ACTIVE_PAGE_TABLE: CpuLocal<KMutex<Option<PagingContext>>,false> = CpuLocal::new();

// = ALLOCATIONS =
// Note: Allocations must be allocated/deallocated manually
// and do not hold a reference to the allocator
// they are more akin to indices
pub struct PageAllocation<PFA: PageFrameAllocator> {
    allocator: LockedPageAllocator<PFA>,
    allocation: PartialPageAllocation,
    metadata: LPAMetadata,
    
    // baseaddr_offset is used in the event that an address passed to allocate_at is not page-aligned
    // the high-level API will handle the difference automatically
    baseaddr_offset: usize,
}
impl<PFA:PageFrameAllocator> PageAllocation<PFA> {
    pub(super) fn new(allocator: LockedPageAllocator<PFA>, allocation: PartialPageAllocation) -> Self {
        Self {
            metadata: *allocator.metadata(),
            allocator: allocator,
            allocation: allocation,
            baseaddr_offset: 0,
        }
    }
    fn assert_pt_tag(&self, allocator: &mut LockedPageAllocatorWriteGuard<PFA,impl core::ops::Deref<Target=PFA>+core::ops::DerefMut<Target=PFA>>){
        // TODO
    }
    
    /* Deliberately leak the allocation, without freeing it.
       Use this instead of core::mem::forget, as this makes sure to drop the Arc<> which means the page table will be dropped when appropriate. */
    pub fn leak(self) -> (PartialPageAllocation, LockedPageAllocator<PFA>, LPAMetadata) {
        use core::{mem::MaybeUninit,ptr};
        let me = MaybeUninit::new(self);
        let me_ptr = me.as_ptr();
        
        // SAFETY: We must make sure to either extract or drop each field within ourselves
        //          as we will not be Drop'd
        unsafe{ (
            ptr::read(&(*me_ptr).allocation),
            ptr::read(&(*me_ptr).allocator ),
            ptr::read(&(*me_ptr).metadata  ),
        )}
    }
    
    /* Find the start address of this allocation in VMem */
    pub fn start(&self) -> usize {
        canonical_addr(self.allocation.start_addr() + self.metadata.offset)
    }
    /* Find the "base" address of this allocation in VMem.
        This will be different from start() if e.g. the address passed to allocate_at was not page aligned.
        start() -> the very start of the allocation (including any padding added to ensure it is page-aligned).
        base() -> the VMem address that was requested. */
    pub fn base(&self) -> usize {
        self.start()+self.baseaddr_offset
    }
    /* Find the end address of this allocation in VMem. (exclusive) */
    pub fn end(&self) -> usize {
        canonical_addr(self.allocation.end_addr() + self.metadata.offset)
    }
    /* The total size of this allocation from start to end (not adjusted by the "base" address) */
    pub fn size(&self) -> usize {
        self.allocation.size()
    }
    /* The length of this allocation following the base (i.e. from base to end)  */
    pub fn length_after_base(&self) -> usize {
        self.size() - self.baseaddr_offset
    }
    
    pub fn set_base_addr(&self, base_addr: usize, flags: PageFlags){
        self.allocator.write_when_active().set_base_addr(self, base_addr-self.baseaddr_offset, flags)
    }
    pub fn set_absent(&self, data: usize){
        self.allocator.write_when_active().set_absent(self, data)
    }
    pub fn flush_tlb(&self){
        self.allocator.write_when_active().invalidate_tlb(self)
    }
    
    pub fn split(self, mid: usize) -> (Self, Self) {
        let allocator = LockedPageAllocator::clone_ref(&self.allocator);
        let result = allocator.write_when_active().split_allocation(self, mid);
        result
    }
}
impl<PFA:PageFrameAllocator> core::ops::Drop for PageAllocation<PFA> {
    fn drop(&mut self){
        self.allocator.write_when_active().dealloc(self);
    }
}
impl<'a,PFA:PageFrameAllocator> From<&'a PageAllocation<PFA>> for &'a PartialPageAllocation {
    fn from(value: &'a PageAllocation<PFA>) -> Self {
        &value.allocation
    }
}

impl<PFA:PageFrameAllocator> core::fmt::Debug for PageAllocation<PFA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageAllocation").field("alloc", &self.allocation).finish()
    }
}

/* Any page allocation, regardless of PFA. */
pub trait AnyPageAllocation: core::fmt::Debug + Send {
    fn start(&self) -> usize;
    fn end(&self) -> usize;
    fn size(&self) -> usize;
    fn set_base_addr(&self, base_addr: usize, flags: PageFlags);
    fn set_absent(&self, data: usize);
    fn flush_tlb(&self);
}
impl<PFA:PageFrameAllocator + Send + Sync> AnyPageAllocation for PageAllocation<PFA> {
    fn start(&self) -> usize { self.start() }
    fn end(&self) -> usize { self.end() }
    fn size(&self) -> usize { self.size() }
    fn set_base_addr(&self, base_addr: usize, flags: PageFlags) { self.set_base_addr(base_addr, flags) }
    fn set_absent(&self, data: usize) { self.set_absent(data) }
    fn flush_tlb(&self) { self.flush_tlb() }
}

pub type TopLevelPageAllocation = PageAllocation<BaseTLPageAllocator>;
pub type TLPageFrameAllocator = BaseTLPageAllocator;