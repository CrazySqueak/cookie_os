//! "interruptions" are things which can unexpectedly change the control flow of a program
//! Examples include maskable interrupts and hidden scheduler_yield calls
//! (nonmaskable interrupts and panics cannot be disabled)

use alloc::vec::Vec;
use core::ptr::null;
use crate::sync::llspin::LLMutex;  // we have to use LLMutexes as KMutex depends on disable_interruptions()
use super::fixedcpulocal::get_fixed_cpu_locals;

pub struct NoInterruptionsGuard(usize);
impl core::ops::Drop for NoInterruptionsGuard {
    fn drop(&mut self) {
        enable_interruptions(self.0)
    }
}

struct NoInterruptionsState {
    interrupt_state: super::arch::enable_interrupts::InterruptState,
}
pub struct NoInterruptionsStateContainer {
    active: bool,
    state: NoInterruptionsState,
}

pub fn disable_interruptions() -> NoInterruptionsGuard {
    // Disable interrupts first of all, to prevent us from being interrupted
    let interrupt_state = super::arch::enable_interrupts::clear_interrupts();
    
    // Disable further interruptions
    // TODO
    
    // Create state object
    let ni_state = NoInterruptionsState {
        interrupt_state,
    };
    // Create state container
    let state = NoInterruptionsStateContainer {
        active: true,
        state: ni_state,
    };
    // Push it
    let mut guard = get_fixed_cpu_locals().current_nointerruptions_state.lock();
    let index = guard.len(); guard.push(state);
    drop(guard);
    // And return a guard
    NoInterruptionsGuard(index)
}
fn enable_interruptions(index: usize) {
    // (disable interrupts so we're not interrupted - it'll be overwritten in a moment anyway)
    let mut interrupt_state = super::arch::enable_interrupts::clear_interrupts();
    
    // Set the given state to "false"
    let mut guard = get_fixed_cpu_locals().current_nointerruptions_state.lock();
    guard[index].active = false;
    
    // And restore any that need it
    while let Some(restore_state) = guard.pop_if(|sc|!sc.active).map(|sc|sc.state) {
        // Other interruptions
        // TODO
        // Interrupts (interrupts are only enabled once, right at the very end)
        interrupt_state = restore_state.interrupt_state;
    }
    drop(guard);
    // Enable interrupts (if applicable)
    super::arch::enable_interrupts::restore_interrupts(&interrupt_state);
}

pub type FCLCurrentNIGuard = LLMutex<Vec<NoInterruptionsStateContainer>>;
#[allow(non_upper_case_globals)]
pub const FCLCurrentNIGuardDefault: FCLCurrentNIGuard = LLMutex::new(Vec::new());