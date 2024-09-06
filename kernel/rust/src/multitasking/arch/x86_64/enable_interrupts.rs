use x86_64::instructions::interrupts as instructions;

pub type InterruptState = bool;

/// Disable all maskable interrupts.
/// Return a value representing the previous state.
#[inline]
pub fn clear_interrupts() -> InterruptState {
    let enabled = instructions::are_enabled();
    if enabled { instructions::disable(); }
    enabled
}
/// Enable all maskable interrupts described by the given state
#[inline]
pub fn restore_interrupts(state: &InterruptState) {
    let state = *state;  // bool is Copy
    if state { instructions::enable(); }
}