
mod interface { extern "C" {
    pub(super) fn is_executing_task() -> bool;
    pub(super) fn spin_yield();
}}

/// Returns true if the scheduler is executing a task
#[inline(always)]
pub fn is_executing_task() -> bool {
    unsafe { interface::is_executing_task() }
}
/// Yield immediately to the scheduler, as if executing a spinlock.
/// Equivalent to yield_to_scheduler(SchedulerCommand::PushBack)
#[inline(always)]
pub fn spin_yield() {
    unsafe { interface::spin_yield() };
}