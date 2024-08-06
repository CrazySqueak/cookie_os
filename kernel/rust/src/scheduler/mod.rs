use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

pub mod task;
pub mod context_switch;

pub use context_switch::{yield_to_scheduler,SchedulerCommand};
pub use task::{Task,TaskType};

static BSP_SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

/// Returns true if the scheduler has initialised on the bootstrap processor
#[inline]
pub fn is_bsp_scheduler_initialised() -> bool {
    BSP_SCHEDULER_READY.load(Ordering::Relaxed)
}

/// Returns true if the scheduler is initialised on the current processor
#[inline]
pub fn is_local_scheduler_ready() -> bool {
    todo!()
}