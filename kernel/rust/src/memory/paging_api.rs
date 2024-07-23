
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

type LockedBaseAllocator = RwLock<BaseTLPageAllocator>;
type ArcBaseAllocator = Arc<LockedBaseAllocator>;
pub struct TopLevelPageAllocator(ArcBaseAllocator);
impl TopLevelPageAllocator {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(BaseTLPageAllocator::new())))
    }
    /* Create another reference to this top-level page table. */
    pub fn clone_ref(x: &Self) -> Self {
        Self(Arc::clone(&x.0))
    }
    
    pub fn write(&self) -> TLPageAllocatorWriteGuard {
        TLPageAllocatorWriteGuard(self.0.write())
    }
    pub fn try_write(&self) -> Option<TLPageAllocatorWriteGuard> {
        match self.0.try_write() {
            Some(guard) => Some(TLPageAllocatorWriteGuard(guard)),
            None => None,
        }
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
        set_active_page_table(table_addr);
        
        // store reference
        let oldpt = _ACTIVE_PAGE_TABLE.lock().replace(Self::clone_ref(&self));
        
        // Decrement reader count on old page table (if applicable)
        // Safety: Since the previous page table was activated using this function,
        //         which leaks a read guard, we can be sure that decrementing the
        //         counter here will be defined, working as if the guard had been dropped.
        // (N.B. we can't simply store the guard due to borrow checker limitations + programmer laziness)
        if let Some(old_table) = oldpt { unsafe {
            old_table.0.force_read_decrement();
        }}
    }
}

pub struct TLPageAllocatorWriteGuard<'a>(RwLockWriteGuard<'a, BaseTLPageAllocator>);

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
impl TLPageAllocatorWriteGuard<'_> {
    //fn _set_addr_inner<PFA: PageFrameAllocator>(allocator: &mut PFA, allocation: &PartialPageAllocation, base_addr: usize){
    //    // entries
    //    let ptable = allocator.get_page_table_mut();
    //    for PAllocEntry{index, offset} in &allocation.entries {
    //        ptable.set_addr(index, base_addr+offset);
    //    }
    //    // sub-allocators
    //    for PAllocSubAlloc{index, offset, alloc} in &allocation.suballocs {
    //    }
    //}
    fn get_page_table(&mut self) -> &mut BaseTLPageAllocator {
        &mut*self.0
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
        ptable.set_addr(index, base_addr+offset);
    });
    
    /* Set the base physical address for the given allocation. This also sets the PRESENT flag automatically. */
    pub fn set_base_addr(&mut self, allocation: &PageAllocation, base_addr: usize){
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        allocation.assert_pt_tag(self);
        unsafe {
            Self::_set_addr_inner(self.get_page_table(), allocation.into(), base_addr);
        }
    }
}

// the currently active page table
static _ACTIVE_PAGE_TABLE: Mutex<Option<TopLevelPageAllocator>> = Mutex::new(None);

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
    pub(super) fn new(allocator: &mut TLPageAllocatorWriteGuard<'_>, allocation: PartialPageAllocation) -> Self {
        Self {
            pagetable_tag: allocator.get_page_table().get_page_table_ptr() as *const u8,
            allocation: allocation,
        }
    }
    
    fn assert_pt_tag(&self, allocator: &mut TLPageAllocatorWriteGuard<'_>){
        assert!(allocator.get_page_table().get_page_table_ptr() as *const u8 == self.pagetable_tag, "Allocation used with incorrect allocator!");
    }
}
impl<'a> From<&'a PageAllocation> for &'a PartialPageAllocation {
    fn from(value: &'a PageAllocation) -> Self {
        &value.allocation
    }
}