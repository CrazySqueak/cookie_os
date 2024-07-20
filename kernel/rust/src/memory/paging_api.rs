
use alloc::sync::Arc;
use spin::rwlock::{RwLock,RwLockReadGuard};
use spin::Mutex;

use super::*;

type BaseTLPageAllocator = arch::TopLevelPageAllocator;
use arch::set_active_page_table;

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
    
    /* Activate this page table. Once active, this page table will be used to map virtual addresses to physical ones.
        Use of Arc ensures that the page table will not be dropped if it is still active.
        DANGER: You MUST follow the proper rules unless you want your program to CRASH UNEXPECTEDLY! This is serious shit!
         * The kernel stack should be at the same virtual memory address in both the old and new tables. It cannot simply be moved to a different one.
         * All heap objects and objects pointed to by pointers should be at the same virtual memory address in both the old and new tables. Any pointers or objects that are not at the same VMem address will cause Undefined Behaviour if accessed (unless the old page table is restored).
         * All kernel code you plan to call must be at the same addresses in both the old and new tables. Most important are INTERRUPT HANDLERS and the PANIC HANDLER (as well as common utilities such as klog). This also includes the activate() function and the function you called it from. (and the static variable that stores the page table)
         The easiest way to achieve the above three points is to map the kernel to the same position in every page table. This is why the kernel lives in the higher half - it should never be necessary to change its location in virtual memory.
         */
    pub unsafe fn activate(&self){
        // activate table
        let allocator = self.0.read();
        let table_addr = ptaddr_virt_to_phys(allocator.get_page_table_ptr() as usize);
        set_active_page_table(table_addr);
        
        // store reference
        let _ = _ACTIVE_PAGE_TABLE.lock().insert(Self::clone_ref(&self));
    }
}
impl core::ops::Deref for TopLevelPageAllocator {
    type Target = ArcBaseAllocator;
    
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// the currently active page table
static _ACTIVE_PAGE_TABLE: Mutex<Option<TopLevelPageAllocator>> = Mutex::new(None);
