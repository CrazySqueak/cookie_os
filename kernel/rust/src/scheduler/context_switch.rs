/* This module is heavily coupled with lowlevel::context_switch, as this is the one that actually contains the scheduler. */
use crate::lowlevel::context_switch as cswitch_impl;
use super::{Task,TaskType};
use crate::sync::Mutex;

pub type StackPointer = cswitch_impl::StackPointer;
pub use cswitch_impl::yield_to_scheduler;

/* The scheduler command given to _cs_push is then passed over to the scheduler.
    It is used to tell the scheduler what to do with the task that just finished. */
pub enum SchedulerCommand {
    /// Push the current task back to the run_queue, and run it again once it's time
    PushBack,
    /// Discard the current task - it has terminated
    Terminate,
}

/* Do not call this function yourself! (unless you know what you're doing). Use yield_to_scheduler instead!
    This function happens after the previous context is saved, but before the next one is loaded. It's this function's job to determine what to run next (and then run it). */
#[inline]
pub fn schedule(command: SchedulerCommand, rsp: StackPointer) -> ! {
    // Update current task
    let mut current_task = _CURRENT_TASK.lock().take().expect("schedule() called but no task currently active?");
    current_task.set_rsp(rsp);
    
    // For now, just print and then resume (testing)
    crate::logging::klog!(Info,ROOT,"Rsp:{:p}",rsp);
    match command {
        SchedulerCommand::PushBack => resume_context(current_task),
        SchedulerCommand::Terminate => {
            crate::logging::klog!(Info,ROOT,"Beep boop halting");
            crate::lowlevel::halt();
        },
    }
}

#[inline]
pub fn resume_context(task: Task) -> !{
    let rsp = task.get_rsp();
    
    // set paging context and stuff if applicable
    
    // set active task
    *_CURRENT_TASK.lock() = Some(task);
    // resume task
    unsafe { cswitch_impl::resume_context(rsp) };
}

// Currently active task
static _CURRENT_TASK: Mutex<Option<Task>> = Mutex::new(None);