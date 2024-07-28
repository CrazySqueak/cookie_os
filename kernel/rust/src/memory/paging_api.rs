
use core::sync::atomic::{AtomicU16,Ordering};
use alloc::sync::Arc;
use spin::rwlock::{RwLock,RwLockReadGuard,RwLockWriteGuard,RwLockUpgradableGuard};
use spin::Mutex;

use super::*;

type BaseTLPageAllocator = arch::TopLevelPageAllocator;
use arch::{set_active_page_table,inval_tlb_pg};

// Note: Flags follow a "union" pattern
// in other words: the combination of all flags should be the most permissive/compatible option
bitflags::bitflags! {
    pub struct PageFlags: u16 {
        // User can access this page
        const USER_ALLOWED = 1<<0;
        // User can write to this page (requires USER_ALLOWED)
        const WRITEABLE = 1<<1;
        // Execution is allowed
        const EXECUTABLE = 1<<2;
        // This page is not present in all page tables, and so should be invalidated when CR3 is updated
        const TLB_NON_GLOBAL = 1<<3;
    }
}

#[derive(Debug,Clone,Copy)]
pub struct LPAMetadata {
    // Offset in VMEM for the start of the page table's jurisdiction. For top-level tables this is 0. For tables nested inside of other tables, this might not be 0
    pub offset: usize,
}
struct LPAInternal<PFA: PageFrameAllocator> {
    // The locked allocator
    lock: RwLock<PFA>,
    // Other metadata
    meta: LPAMetadata,
    // The number of times this is "active" - usually 1 for global tables, or <number of CPUs table is active for> for top-level contexts.
    active_count: AtomicU16,
}
impl<PFA: PageFrameAllocator> LPAInternal<PFA> {
    fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self {
            lock: RwLock::new(alloc),
            meta: meta,
            active_count: AtomicU16::new(0),
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
    
    /* Lock the allocator for reading until _end_active is called.
    This is intended to be used when the page table is possibly being read/cached by the CPU, as when locked by _begin_active, writes via write_when_active are still possible (as they flush the TLB). */
    pub(super) fn _begin_active(&self) -> &PFA {
        // Lock
        let alloc = RwLockReadGuard::leak(self.0.read());
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        // Increment active count
        self.0.active_count.fetch_add(1, Ordering::Acquire);
        // Return
        alloc
    }
    pub(super) unsafe fn _end_active(&self){
        // Decrement active count
        self.0.active_count.fetch_sub(1, Ordering::Release);
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        // Unlock
        self.0.force_read_decrement();
    }
    
    pub fn read(&self) -> RwLockReadGuard<PFA> {
        self.0.read()
    }
    
    pub fn write(&self) -> LPageAllocatorRWLWriteGuard<PFA> {
        LockedPageAllocatorWriteGuard{guard: self.0.write(), meta: self.0.metadata()}
    }
    pub fn try_write(&self) -> Option<LPageAllocatorRWLWriteGuard<PFA>> {
        match self.0.try_write() {
            Some(guard) => Some(LockedPageAllocatorWriteGuard{guard, meta: self.0.metadata()}),
            None => None,
        }
    }
    
    /* Write to a page table that is currently active, provided there are no other read/write locks.
        Writes using this guard will automatically invalidate the TLB entries as needed for the current CPU (but currently not OTHERS!!! TODO).*/
    pub fn write_when_active(&self) -> LPageAllocatorUnsafeWriteGuard<PFA> {  // TODO: With TLB inval
        // Acquire an upgradable read guard, to ensure that A. there are no writers, and B. there are no new readers while we're determining what to do
        let upgradable = self.0.upgradeable_read();
        core::sync::atomic::compiler_fence(Ordering::SeqCst);
        
        // Ensure reader count matches our active_count
        /* SAFETY / SYNC:
           Since no new readers can be allocated, reader_count will not have increased during this time
           Since reader_count is incremented before active_count, if a race occurs during _begin_active, then reader_count > active_count, which means a lock is not acquired and we must wait and try again.
           Similarly, active_count must be decremented before reader_count in _end_active. If a race occurs during _end_active, then reader_count > active_count, which means a lock is not acquired blah blah blah
           We also read reader_count before active_count, to ensure that in a race with _end_active, we will end up with reader_count > active_count (since active_count is decremented first).
           
           Note: Reader count was incremented by 1 when we took the upgradeable guard - so actually we should compare against active_count+1.
        */
        loop {
            let reader_count = self.0.reader_count();
            core::sync::atomic::compiler_fence(Ordering::SeqCst);
            let active_count = self.0.active_count.load(Ordering::Acquire);
            if reader_count <= (active_count+1).into() {
                // Force-upgrade and return the write lock
                // We cannot upgrade normally because that would require there to be no readers
                // Whereas here we've ensured that the readers are all CPU/TLB readers or something
                // So we want to forcibly create an upgraded version
                let write_guard = unsafe { ForcedUpgradeGuard::new(&self.0, upgradable) };
                return LockedPageAllocatorWriteGuard{guard: write_guard, meta: self.0.metadata()};
            } else {
                // Relax
                // 🛏☺☕ ahhh
                use spin::RelaxStrategy;
                spin::relax::Spin::relax();
            }
        }
    }
}

pub struct PagingContext(LockedPageAllocator<BaseTLPageAllocator>);
impl PagingContext {
    pub fn new() -> Self {
        klog!(Debug, "memory.paging.context", "Creating new paging context.");
        let mut allocator = BaseTLPageAllocator::new();
        // Add global pages
        for (i,addr) in global_pages::GLOBAL_TABLE_PHYSADDRS.iter().enumerate(){
            // SAFETY: See documentation for put_global_table and GLOBAL_TABLE_PHYSADDRS
            unsafe{ allocator.put_global_table(global_pages::GLOBAL_PAGES_START_IDX+i,*addr); }
        }
        // Return
        Self(LockedPageAllocator::new(allocator, LPAMetadata { offset: 0 }))
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
        klog!(Info, "memory.paging.context", "Switching active context to 0x{:x}", table_addr);
        set_active_page_table(table_addr);
        
        // store reference
        let oldpt = _ACTIVE_PAGE_TABLE.lock().replace(Self::clone_ref(&self));
        
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
pub struct LockedPageAllocatorWriteGuard<PFA: PageFrameAllocator, GuardT>
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA>
    { guard: GuardT, meta: LPAMetadata }
//pub type TLPageAllocatorWriteGuard<'a> = LockedPageAllocatorWriteGuard<'a, BaseTLPageAllocator>;
pub type LPageAllocatorRWLWriteGuard<'a, PFA> = LockedPageAllocatorWriteGuard<PFA, RwLockWriteGuard<'a, PFA>>;
pub type LPageAllocatorUnsafeWriteGuard<'a, PFA> = LockedPageAllocatorWriteGuard<PFA, ForcedUpgradeGuard<'a, PFA>>;

macro_rules! ppa_define_foreach {
    // Note: body takes four variables from the outside: allocator, ptable, index, and offset (as well as any other args they specify).
    ($($fkw:ident)*: $fnname: ident, $pfaname:ident:&mut PFA, $allocname:ident:&PPA, $ptname:ident:&mut IPT, $idxname:ident:usize, $offname:ident:usize, $($argname:ident:$argtype:ty),*, $body:block) => {
        $($fkw)* fn $fnname($pfaname: &mut impl PageFrameAllocator, $allocname: &PartialPageAllocation, $($argname:$argtype),*){
            // entries
            let $ptname = $pfaname.get_page_table_mut();
            for &PAllocEntry{index:$idxname, offset:$offname} in &$allocname.entries {
                $body;
            }
            // sub-allocators
            for PAllocSubAlloc{index, offset, alloc: suballocation} in &$allocname.suballocs {
                let suballocator = $pfaname.get_suballocator_mut(*index).expect("Allocation expected sub-allocator but none was found!");
                Self::$fnname(suballocator,suballocation, $($argname),*);  // TODO: offset?
            }
        }
    }
}
impl<PFA: PageFrameAllocator, GuardT> LockedPageAllocatorWriteGuard<PFA, GuardT> 
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA>
  {
    fn get_page_table(&mut self) -> &mut PFA {
        &mut*self.guard
    }
    
    // Allocating
    pub fn allocate(&mut self, size: usize) -> Option<PageAllocation> {
        let allocator = self.get_page_table();
        let allocation = allocator.allocate(size)?;
        Some(PageAllocation::new(self, allocation))
    }
    pub fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PageAllocation> {
        // Convert the address
        // 1. Convert from canonical vmem addr to 0-extended (so it can be converted to an index)
        let addr = addr&0x0000ffff_ffffffff;
        // 2. We subtract the offset from the vmem address (as we need it to be relative to the start of our table)
        let rel_addr = addr.checked_sub(self.meta.offset).unwrap_or_else(||panic!("Cannot allocate memory before the start of the page! addr=0x{:x} page_start=0x{:x}",addr,self.meta.offset));
        
        // Allocate
        let allocator = self.get_page_table();
        let allocation = allocator.allocate_at(rel_addr, size)?;
        Some(PageAllocation::new(self, allocation))
    }
    
    // Managing allocations
    ppa_define_foreach!(unsafe: _set_addr_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, base_addr: usize, {
        ptable.set_huge_addr(index, base_addr+offset);
    });
    
    /* Set the base physical address for the given allocation. This also sets the PRESENT flag automatically. */
    pub fn set_base_addr(&mut self, allocation: &PageAllocation, base_addr: usize){
        allocation.assert_pt_tag(self);
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        unsafe {
            Self::_set_addr_inner(self.get_page_table(), allocation.into(), base_addr);
        }
    }
    
    ppa_define_foreach!(unsafe: _set_missing_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, data: usize, {
        ptable.set_absent(index, data);
    });
    /* Set the given allocation as absent (not in physical memory). */
    pub fn set_absent(&mut self, allocation: &PageAllocation, data: usize){
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        allocation.assert_pt_tag(self);
        unsafe {
            Self::_set_missing_inner(self.get_page_table(), allocation.into(), data);
        }
    }
    
    ppa_define_foreach!(: _inval_tlb_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, vmem_start: usize, {
        inval_tlb_pg(vmem_start + offset)
    });
    /* Invalidate the TLB entries for the given allocation (on the current CPU).
        Note: No check is performed to ensure that the allocation is correct nor that this page table is active, as the only consequence (provided all other code handling Page Tables / TLB is correct) is a performance hit from the unnecessary INVLPG operations + the resulting cache misses.
        Note: Using this method is unnecessary yourself. Usually it is provided by write_when_active or similar. */
    pub fn invalidate_tlb(&mut self, allocation: &PageAllocation){
        let vmem_offset = self.meta.offset;
        Self::_inval_tlb_inner(self.get_page_table(), allocation.into(), vmem_offset);
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
        let ptr = &mut *lock.as_mut_ptr();
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
// the currently active page table
static _ACTIVE_PAGE_TABLE: Mutex<Option<PagingContext>> = Mutex::new(None);

// = ALLOCATIONS =
// Note: Allocations must be allocated/deallocated manually
// and do not hold a reference to the allocator
// they are more akin to indices
pub struct PageAllocation {
    // Pagetable tag is used to check that the correct page table is used or something?
    pagetable_tag: *const u8,
    allocation: PartialPageAllocation,
}
impl PageAllocation {
    pub(super) fn new<PFA:PageFrameAllocator>(allocator: &mut LockedPageAllocatorWriteGuard<PFA,impl core::ops::Deref<Target=PFA>+core::ops::DerefMut<Target=PFA>>, allocation: PartialPageAllocation) -> Self {
        Self {
            pagetable_tag: allocator.get_page_table().get_page_table_ptr() as *const u8,
            allocation: allocation,
        }
    }
    
    fn assert_pt_tag<PFA:PageFrameAllocator>(&self, allocator: &mut LockedPageAllocatorWriteGuard<PFA,impl core::ops::Deref<Target=PFA>+core::ops::DerefMut<Target=PFA>>){
        assert!(allocator.get_page_table().get_page_table_ptr() as *const u8 == self.pagetable_tag, "Allocation used with incorrect allocator!");
    }
}
impl<'a> From<&'a PageAllocation> for &'a PartialPageAllocation {
    fn from(value: &'a PageAllocation) -> Self {
        &value.allocation
    }
}