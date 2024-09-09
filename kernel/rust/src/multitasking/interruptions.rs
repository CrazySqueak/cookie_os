//! "interruptions" are things which can unexpectedly change the control flow of a program
//! Examples include maskable interrupts and hidden scheduler_yield calls
//! (nonmaskable interrupts and panics cannot be disabled)

use alloc::vec::Vec;
use core::ptr::null;
use crate::sync::llspin::LLMutex;  // we have to use LLMutexes as KMutex depends on disable_interruptions()
use super::fixedcpulocal::fixed_cpu_local;
use core::sync::atomic::{AtomicBool,Ordering};

pub struct NoInterruptionsGuard(usize);
impl core::ops::Drop for NoInterruptionsGuard {
    fn drop(&mut self) {
        enable_interruptions(self.0)
    }
}

struct NoInterruptionsState {
    interrupt_state: super::arch::enable_interrupts::InterruptState,
    yield_state: bool,
}
pub struct NoInterruptionsStateContainer {
    active: bool,
    state: NoInterruptionsState,
}

pub fn disable_interruptions() -> NoInterruptionsGuard {
    // Disable interrupts first of all, to prevent us from being interrupted
    let interrupt_state = super::arch::enable_interrupts::clear_interrupts();
    
    // Disable scheduler_yield
    let yield_state = SCHEDULER_YIELD_DISABLED.swap(true, Ordering::AcqRel);  // idk what ordering to use
    
    // Create state object
    let ni_state = NoInterruptionsState {
        interrupt_state,
        yield_state,
    };
    // Create state container
    let state = NoInterruptionsStateContainer {
        active: true,
        state: ni_state,
    };
    // Push it
    let mut guard = CURRENT_NOINTERRUPTIONS_STATE.lock();
    let index = guard.len(); guard.push(state);
    drop(guard);
    // And return a guard
    NoInterruptionsGuard(index)
}
fn enable_interruptions(index: usize) {
    // (disable interrupts so we're not interrupted - it'll be overwritten in a moment anyway)
    let mut interrupt_state = super::arch::enable_interrupts::clear_interrupts();
    
    // Set the given state to "false"
    let mut guard = CURRENT_NOINTERRUPTIONS_STATE.lock();
    guard[index].active = false;
    
    // And restore any that need it
    while let Some(restore_state) = guard.pop_if(|sc|!sc.active).map(|sc|sc.state) {
        // Scheduler yield
        SCHEDULER_YIELD_DISABLED.store(restore_state.yield_state, Ordering::AcqRel);
        // Interrupts (interrupts are only enabled once, right at the very end)
        interrupt_state = restore_state.interrupt_state;
    }
    drop(guard);
    // Enable interrupts (if applicable)
    super::arch::enable_interrupts::restore_interrupts(&interrupt_state);
}

fixed_cpu_local!(fixedcpulocal static CURRENT_NOINTERRUPTIONS_STATE: LLMutex<Vec<NoInterruptionsStateContainer>> = LLMutex::new(Vec::new()));
fixed_cpu_local!(fixedcpulocal static SCHEDULER_YIELD_DISABLED: AtomicBool = AtomicBool::new(false));
// pub type FCLCurrentNIGuard = LLMutex<Vec<NoInterruptionsStateContainer>>;
// #[allow(non_upper_case_globals)]
// pub const FCLCurrentNIGuardDefault: FCLCurrentNIGuard = LLMutex::new(Vec::new());

/// Return true if scheduler_yield has been disabled by disable_interruptions
pub fn is_sched_yield_disabled() -> bool {
    SCHEDULER_YIELD_DISABLED.load(Ordering::Relaxed)
}