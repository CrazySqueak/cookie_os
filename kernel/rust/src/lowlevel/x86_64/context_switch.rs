use crate::scheduler::context_switch as cswitch_api;

extern "sysv64" {
    /* Trigger a context switch, invoking the scheduler in order to do so.
        From the perspective of the current thread, this function only returns when the thread is resumed.
        Before then, the scheduler may use the CPU for other threads. */
    fn _cs_push(command: *mut u8) -> ();
    /* Trigger the "load" portion of a context switch. This function never returns and the current state is discarded.
        It is recommended to save the current state with _cs_push, and have the scheduler call _cs_pop instead. */
    fn _cs_pop(rsp: *const u8) -> !;
    /* Initialise a new stack at the given address, which calls the given entry point.
        For the entry point: The stack starts empty. Callee-saved registers (RBX,R12-15) are zeroed. The value of caller-saved registers are undefined. */
    pub fn _cs_new(entrypoint: extern "sysv64" fn() -> !, stack: *const u8) -> *const u8;
}

pub type StackPointer = *const u8;

use alloc::boxed::Box;  // box is used for passing across the command

/* Scheduler callback triggered by _cs_push. */
#[no_mangle]
unsafe extern "sysv64" fn contextswitch_scheduler_cb(command: *mut cswitch_api::SchedulerCommand, rsp: StackPointer) -> ! {
    cswitch_api::schedule(Box::into_inner(Box::from_raw(command)), rsp);
}

/* Begin a context switch by calling _cs_push, yielding to the scheduler. Return once this thread resumes. */
#[inline]
pub fn yield_to_scheduler(command: cswitch_api::SchedulerCommand) -> () {
    let command_ptr = Box::into_raw(Box::new(command));
    unsafe { _cs_push(command_ptr as *mut u8) }
}

/* Finish a context switch by resuming with the given stack pointer. */
#[inline]
pub unsafe fn resume_context(rsp: StackPointer) -> ! {
    _cs_pop(rsp)
}