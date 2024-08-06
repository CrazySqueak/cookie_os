/* This module is heavily coupled with lowlevel::context_switch, as this is the one that actually contains the scheduler. */
use crate::lowlevel::context_switch as cswitch_impl;
use super::{Task,TaskType};
use crate::sync::{Mutex,AlwaysPanic};
use alloc::collections::VecDeque;
use crate::logging::klog;

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
    let mut current_task = get_current_task().take().expect("schedule() called but no task currently active?");
    current_task.set_rsp(rsp);
    
    // Push previous task back onto the run queue
    match &command {
        SchedulerCommand::Terminate => {
            // Terminate the task
            klog!(Debug, SCHEDULER, "Terminating task: {}", current_task.task_id);
            drop(current_task)
        }
        
        _ => {
            // Push back onto run queue
            klog!(Debug, SCHEDULER, "Suspending task: {}", current_task.task_id);
            get_run_queue().push_back(current_task);
        }
    }
    
    // Pick the next task off of the run queue
    loop {
        let next_task: Option<Task>;
        match &command {
            _ => {
                // Take next task
                next_task = get_run_queue().pop_front();
            }
        }
        
        if let Some(next_task) = next_task {
            klog!(Debug, SCHEDULER, "Resuming task: {}", next_task.task_id);
            resume_context(next_task)
        }
    }
}

/* Resume the requested task, discarding the current one (if any). */
#[inline]
pub fn resume_context(task: Task) -> !{
    let rsp = task.get_rsp();
    
    // set paging context and stuff if applicable
    
    // set active task
    *get_current_task() = Some(task);
    // resume task
    unsafe { cswitch_impl::resume_context(rsp) };
}

/* Initialise the scheduler for the current CPU, before creating a kernel task to represent the current stack.
    Once this has been called, it is ok to call yield_to_scheduler.
    (calling this again will discard a large amount of the scheduler's state for the current CPU, so uh, don't)*/
pub fn init_scheduler(){
    // initialise run queue? (idk TODO)
    
    // Initialise task
    // Note: resuming the task is undefined (however that is the same for all "currently active tasks" - as they must be paused first)
    let task = unsafe { Task::new_with_rsp(TaskType::KernelTask, core::ptr::null_mut()) };
    let task_id = task.task_id;
    // Set current task
    *get_current_task() = Some(task);
    
    // All gucci :)
    // log message
    klog!(Info, SCHEDULER, "Initialised scheduler on CPU {}. Bootstrapper task has become task {}.", 0, task_id);
    // Signal that scheduler is online
    super::BSP_SCHEDULER_READY.store(true,core::sync::atomic::Ordering::Release);
}

/* Push a new task to the current scheduler's run queue. */
pub fn push_task(task: Task){
    klog!(Debug, SCHEDULER, "Pushing new task: {}", task.task_id);
    get_run_queue().push_back(task);
}

// Currently active task & run queue
static _CURRENT_TASK: Mutex<Option<Task>, AlwaysPanic> = Mutex::new(None);
static _RUN_QUEUE: Mutex<VecDeque<Task>, AlwaysPanic> = Mutex::new(VecDeque::new());  // TODO replace with a better run queue system

#[inline(always)]
pub(super) fn get_current_task() -> crate::sync::MutexGuard<'static, Option<Task>> { _CURRENT_TASK.lock() }
#[inline(always)]
pub(super) fn get_run_queue() -> crate::sync::MutexGuard<'static, VecDeque<Task>> { _RUN_QUEUE.lock() }