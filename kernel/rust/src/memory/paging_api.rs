
use alloc::sync::Arc;
use spin::rwlock::{RwLock,RwLockReadGuard,RwLockWriteGuard};
use spin::Mutex;

use super::*;

type BaseTLPageAllocator = arch::TopLevelPageAllocator;
use arch::set_active_page_table;

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
}
impl<PFA: PageFrameAllocator> LPAInternal<PFA> {
    fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self {
            lock: RwLock::new(alloc),
            meta: meta,
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
    
    pub fn read(&self) -> RwLockReadGuard<PFA> {
        self.0.read()
    }
    
    pub fn write(&self) -> LockedPageAllocatorWriteGuard<PFA> {
        LockedPageAllocatorWriteGuard{guard: self.0.write(), meta: self.0.metadata()}
    }
    pub fn try_write(&self) -> Option<LockedPageAllocatorWriteGuard<PFA>> {
        match self.0.try_write() {
            Some(guard) => Some(LockedPageAllocatorWriteGuard{guard, meta: self.0.metadata()}),
            None => None,
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
        let allocator = RwLockReadGuard::leak(self.0.read());
        
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
            old_table.0.0.force_read_decrement();
        }}
    }
}
impl core::ops::Deref for PagingContext {
    type Target = LockedPageAllocator<BaseTLPageAllocator>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct LockedPageAllocatorWriteGuard<'a, PFA: PageFrameAllocator>{ guard: RwLockWriteGuard<'a, PFA>, meta: LPAMetadata }
pub type TLPageAllocatorWriteGuard<'a> = LockedPageAllocatorWriteGuard<'a, BaseTLPageAllocator>;

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
impl<PFA: PageFrameAllocator> LockedPageAllocatorWriteGuard<'_, PFA> {
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
        let allocator = self.get_page_table();
        let allocation = allocator.allocate_at(addr, size)?;
        Some(PageAllocation::new(self, allocation))
    }
    
    // Managing allocations
    ppa_define_foreach!(unsafe: _set_addr_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, base_addr: usize, {
        ptable.set_huge_addr(index, base_addr+offset);
    });
    
    /* Set the base physical address for the given allocation. This also sets the PRESENT flag automatically. */
    pub fn set_base_addr(&mut self, allocation: &PageAllocation, base_addr: usize){
        allocation.assert_pt_tag(self);
        // N.B. If offset is non-zero, we must convert from an absolute vmem address to an address relative to the start of the table
        let vmem_rel_addr = base_addr - self.meta.offset;
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        unsafe {
            Self::_set_addr_inner(self.get_page_table(), allocation.into(), vmem_rel_addr);
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
}

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
    pub(super) fn new(allocator: &mut LockedPageAllocatorWriteGuard<'_,impl PageFrameAllocator>, allocation: PartialPageAllocation) -> Self {
        Self {
            pagetable_tag: allocator.get_page_table().get_page_table_ptr() as *const u8,
            allocation: allocation,
        }
    }
    
    fn assert_pt_tag(&self, allocator: &mut LockedPageAllocatorWriteGuard<'_,impl PageFrameAllocator>){
        assert!(allocator.get_page_table().get_page_table_ptr() as *const u8 == self.pagetable_tag, "Allocation used with incorrect allocator!");
    }
}
impl<'a> From<&'a PageAllocation> for &'a PartialPageAllocation {
    fn from(value: &'a PageAllocation) -> Self {
        &value.allocation
    }
}