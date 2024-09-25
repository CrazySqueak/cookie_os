use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

pub mod task;
pub mod scheduler;
pub mod util;
pub mod interruptions;

crate::arch_specific_module!(mod arch);

pub use scheduler::{yield_to_scheduler,terminate_current_task,SchedulerCommand,get_executing_task_id};
pub use task::{Task,TaskType};
pub use interruptions::{without_interruptions,are_interruptions_disabled};

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
pub fn is_executing_task() -> bool {
    return is_bsp_scheduler_initialised() && scheduler::is_executing_task()
}

