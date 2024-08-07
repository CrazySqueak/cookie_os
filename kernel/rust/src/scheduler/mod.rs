use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

pub mod task;
pub mod context_switch;
pub mod multicore;

pub use context_switch::{yield_to_scheduler,terminate_current_task,SchedulerCommand,on_clock_tick};
pub use task::{Task,TaskType};

static BSP_SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

/// Returns true if the scheduler has initialised on the bootstrap processor
#[inline]
pub fn is_bsp_scheduler_initialised() -> bool {
    BSP_SCHEDULER_READY.load(Ordering::Relaxed)
}

/** Returns true if the scheduler on the current processor is ready for you to yield.
    This being false does not mean that your scheduler is not initialised, just that it won't accept a yield at this point.
    For example, code running as part of the scheduler in-between tasks will see this as false instead of true.
    Conversely, code running as part of a job will likely never see this return false.
    Code that is written to be used by both the scheduler and by jobs should make sure to check this function's return value before yielding,
        as yielding when this function returns false will panic (i mean, if the scheduler tries to yield to itself what else are you supposed to do? cause a deadlock?).
*/
#[inline]
pub fn is_scheduler_ready() -> bool {
    return is_bsp_scheduler_initialised() && context_switch::get_current_task().is_some()
}