
/* The scheduler command given to _cs_push is then passed over to the scheduler.
    It is used to tell the scheduler what to do with the task that just finished. */
#[repr(u8)]
pub enum SchedulerCommand {
    /// Push the current task back to the run_queue, and run it again once it's time
    PushBack,
    /// Discard the current task - it has terminated. (this does not perform unwinding).
    /// It is preferred to use terminate_current_task or similar instead of yield_to_scheduler(Terminate) where possible.
    Terminate,
    /// Sleep for the requested number of PIT ticks
    SleepNTicks(usize),
}
pub type StackPointer = *const u8;

extern "sysv64" {
    pub fn schedule(command: SchedulerCommand, rsp: StackPointer) -> !;
}