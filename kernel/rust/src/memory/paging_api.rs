
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
// TODO

// the currently active page table
static _ACTIVE_PAGE_TABLE: Mutex<Option<TopLevelPageAllocator>> = Mutex::new(None);
