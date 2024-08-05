/* This module is heavily coupled with lowlevel::context_switch, as this is the one that actually contains the scheduler. */
use crate::lowlevel::context_switch as cswitch_impl;

pub type StackPointer = cswitch_impl::StackPointer;
pub use cswitch_impl::yield_to_scheduler;

use cswitch_impl::resume_context;

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
    todo!()
}