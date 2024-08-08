/* This module is heavily coupled with lowlevel::context_switch, as this is the one that actually contains the scheduler. */
use crate::lowlevel::{context_switch as cswitch_impl};
use super::{Task,TaskType};
use alloc::collections::VecDeque;
use crate::logging::klog;
use crate::sync::cpulocal::{CpuLocal,CpuLocalGuard,CpuLocalLockedOption,CpuLocalLockedItem};

// Currently active task & run queue
struct SchedulerState {
    run_queue: VecDeque<Task>, // TODO replace with a better run queue system
    
    // Attempting to drop the most recent task causes an exception because its stack may still be in use
    // so instead we store it here V and drop it later on
    deferred_drop: Option<Task>,
}
impl core::default::Default for SchedulerState {
    fn default() -> Self {
        Self {
            run_queue: VecDeque::new(),
            deferred_drop: None,
        }
    }
}
// current_task is stored separately to the rest of the state as it is commonly accessed by logging methods,
// and usually isn't held for very long. If it was part of _SCHEDULER_STATE, then logging during with_scheduler_state! would cause a deadlock
static _CURRENT_TASK: CpuLocalLockedOption<Task> = CpuLocalLockedOption::new();
static _SCHEDULER_STATE: CpuLocalLockedItem<SchedulerState> = CpuLocal::new();

pub type StackPointer = cswitch_impl::StackPointer;
pub use cswitch_impl::yield_to_scheduler;
/* Terminate the current task. This is akin to calling yield_to_scheduler(Terminate), but returns the "!" type to hint that it cannot resume afterwards. */
#[inline]
pub fn terminate_current_task() -> ! {
    yield_to_scheduler(SchedulerCommand::Terminate);
    unreachable!();  // the scheduler will drop the task when yield is called with the Terminate command
}

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
    _SCHEDULER_STATE.mutate(|state|{
        // Update current task
        let mut current_task = _CURRENT_TASK.take().expect("schedule() called but no task currently active?");
        current_task.set_rsp(rsp);
        
        // Push previous task back onto the run queue
        match &command {
            SchedulerCommand::Terminate => {
                // Terminate the task
                klog!(Debug, SCHEDULER, "Terminating task: {}", current_task.task_id);
                state.deferred_drop = Some(current_task)
            }
            
            _ => {
                // Push back onto run queue
                klog!(Debug, SCHEDULER, "Suspending task: {}", current_task.task_id);
                state.run_queue.push_back(current_task);
            }
        }
    });
    
    // Pick the next task off of the run queue
    loop {
        let next_task: Option<Task> = _SCHEDULER_STATE.mutate(|state|{
            match &command {
                _ => {
                    // Take next task
                    state.run_queue.pop_front()
                }
            }
        });
        
        if let Some(next_task) = next_task {
            klog!(Debug, SCHEDULER, "Resuming task: {}", next_task.task_id);
            resume_context(next_task)
        } else {
            // No tasks to do
            // spin for now
            use spin::RelaxStrategy;
            spin::relax::Spin::relax();
        }
    }
}

/* Resume the requested task, discarding the current one (if any). */
#[inline]
pub fn resume_context(task: Task) -> !{
    let rsp = task.get_rsp();
    
    // set paging context and stuff if applicable
    
    // set active task
    {
        _CURRENT_TASK.insert(task);
    }  // <- lock gets dropped here
    // resume task
    unsafe { cswitch_impl::resume_context(rsp) };
}

/* Initialise the scheduler for the current CPU, before creating a kernel task to represent the current stack.
    Once this has been called, it is ok to call yield_to_scheduler.
    (calling this again will discard a large amount of the scheduler's state for the current CPU, so uh, don't)*/
pub fn init_scheduler(){
    _SCHEDULER_STATE.mutate(|state|{
        // initialise run queue?
        state.run_queue.reserve(8);
        
        // Initialise task
        // Note: resuming the task is undefined (however that is the same for all "currently active tasks" - as they must be paused first)
        let task = unsafe { Task::new_with_rsp(TaskType::KernelTask, core::ptr::null_mut(), None) };
        let task_id = task.task_id;
        // Set current task
        _CURRENT_TASK.insert(task);
        
        // All gucci :)
        // log message
        klog!(Info, SCHEDULER, "Initialised scheduler on CPU {}. Bootstrapper task has become task {}.", 0, task_id);
        // Signal that scheduler is online
        super::BSP_SCHEDULER_READY.store(true,core::sync::atomic::Ordering::Release);
    });
}

/* Push a new task to the current scheduler's run queue. */
pub fn push_task(task: Task){
    klog!(Debug, SCHEDULER, "Pushing new task: {}", task.task_id);
    _SCHEDULER_STATE.mutate(|state|state.run_queue.push_back(task))
}

//#[inline(always)]
//pub(super) fn get_current_task() -> CpuLocalGuard<'static,Option<Task>> { let cg = _CURRENT_TASK.get() }
//#[inline(always)]
//pub(super) fn get_run_queue() -> CpuLocalGuard<'static,VecDeque<Task>> { let cg = _RUN_QUEUE.get() }

/* Returns true if the scheduler is currently executing a task. Returns false otherwise (i.e. it's instead executing bootstrap or scheduler code). */
#[inline(always)]
pub fn is_executing_task() -> bool {
    _CURRENT_TASK.inspect(|ot|ot.is_some())
}
/* Get the ID of the current task, or None if the scheduler is running right now instead of a specific task. */
#[inline(always)]
pub fn get_executing_task_id() -> Option<usize> {
    _CURRENT_TASK.inspect(|ot|ot.as_ref().map(|t|t.task_id))
}