//! "interruptions" are things which can unexpectedly change the control flow of a program
//! Examples include maskable interrupts and hidden scheduler_yield calls
//! (nonmaskable interrupts and panics cannot be disabled)

use alloc::sync::{Arc,Weak};
use core::ptr::null;
use spin::Mutex;
use super::fixedcpulocal::get_fixed_cpu_locals;

/// A guard that means that no interruptions may happen
/// It stores the previous state, as well as a reference to the previous guard to ensure that state is restored in the correct order
pub struct NoInterruptionsGuard {
    interrupt_state: super::arch::enable_interrupts::InterruptState,
    
    enclosing_guard: Option<Arc<Self>>,
}
impl NoInterruptionsGuard {
    /// Disable interruptions
    fn new() -> Arc<Self> {
        // Disable interrupts first of all, to prevent us from being interrupted
        let interrupt_state = super::arch::enable_interrupts::clear_interrupts();
        
        // Disable further interruptions
        // TODO
        
        // Take a reference to the enclosing guard
        let mut current_ni_guard = get_fixed_cpu_locals().current_ni_guard.lock();
        let enclosing_guard = current_ni_guard.as_ref().and_then(Weak::upgrade);
        
        // Create ourselves
        let this = Arc::new(Self {
            interrupt_state,
            
            enclosing_guard,
        });
        
        // Mark ourselves as the new enclosing guard
        *current_ni_guard = Some(Arc::downgrade(&this));
        // And return
        this
    }
}
impl core::ops::Drop for NoInterruptionsGuard {
    fn drop(&mut self){
        // Update the current no-interruptions guard (replacing ourselves with our enclosing guard)
        {
            let enclosing_guard = self.enclosing_guard.as_ref().map(|eg|Arc::downgrade(eg));
            let mut current_ni_guard = get_fixed_cpu_locals().current_ni_guard.lock();
            let self_weak = core::mem::replace(&mut*current_ni_guard, enclosing_guard);
            assert!(self_weak.map(|w|w.as_ptr()).unwrap_or_else(null) == ((self as *mut Self) as *const Self));
        }
        
        // Restore state
        // TODO
        super::arch::enable_interrupts::restore_interrupts(&self.interrupt_state);
    }
}
// We use Weaks to ensure that Drop gets called once the code no longer references the guard
pub type FCLCurrentNIGuard = Mutex<Option<Weak<NoInterruptionsGuard>>>;
#[allow(non_upper_case_globals)]
pub const FCLCurrentNIGuardDefault: FCLCurrentNIGuard = Mutex::new(None);