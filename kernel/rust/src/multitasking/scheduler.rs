
use super::arch::{context_switch as cswitch_impl};
use super::task::{Task,TaskType};
use alloc::collections::VecDeque;
macro_rules! klog { ($($x:tt)*)=>{} }//FIXME use crate::logging::klog;
use super::cpulocal::CpuLocal;
use super::fixedcpulocal::{get_fixed_cpu_locals,fixed_cpu_local};
use super::interruptions::{disable_interruptions,NoInterruptionsGuard};

//use crate::sync::kspin::{KMutexRaw,KRwLockRaw};
use crate::sync::kspin::{KMutex,KMutexGuard};
use core::sync::atomic::{AtomicUsize,AtomicBool,Ordering};
use crate::sync::{yspin::YMutexGuard,waitlist::WaitingListEntry};

// Currently active task & run queue
struct SchedulerState {
    run_queue: VecDeque<Task>, // TODO replace with a better run queue system
    
    /// Some internal IO tasks are currently put here when they need to sleep for a while
    /// This will eventually be superseded by a proper "sleep" system
    sleeping: VecDeque<(Task,usize)>,
    
    // We can't drop tasks in scheduler code because the memory allocators use Y/WLocks
    deferred_drop: alloc::vec::Vec<Task>,
}
impl core::default::Default for SchedulerState {
    fn default() -> Self {
        Self {
            run_queue: VecDeque::new(),
            sleeping: VecDeque::new(),
            deferred_drop: alloc::vec::Vec::new(),
        }
    }
}
// current_task is stored separately to the rest of the state as it is commonly accessed by logging methods,
// and usually isn't held for very long. If it was part of _SCHEDULER_STATE, then logging during with_scheduler_state! would cause a deadlock
static _CURRENT_TASK: CpuLocal<KMutex<Option<Task>>,false> = CpuLocal::new();

static _SCHEDULER_STATE: CpuLocal<KMutex<SchedulerState>,true> = CpuLocal::new();
static _SCHEDULER_TICKS: CpuLocal<AtomicUsize,false> = CpuLocal::new();

// _IS_EXECUTING_TASK is a lock-free heuristic for checking if a task is not currently executing, even if the scheduler is not initialised yet on this CPU or if the scheduler is deadlocked
// It is false when scheduler/bootstrap code is executing, and is true starting right before resume_context is called.
// It is only intended as a heuristic. If you intend to interact with tasks properly, use a standard lock acquire and match statement.
// static _IS_EXECUTING_TASK: CpuLocal<AtomicBool,KRwLockRaw> = CpuLocal::new();
fixed_cpu_local!(fixedcpulocal static _IS_EXECUTING_TASK: AtomicBool = AtomicBool::new(false));

pub type StackPointer = cswitch_impl::StackPointer;
pub use cswitch_impl::yield_to_scheduler;
/* Terminate the current task. This is akin to calling yield_to_scheduler(Terminate), but returns the "!" type to hint that it cannot resume afterwards.
    This currently does not unwind the stack, so any objects you store in the stack will not be dropped. However, this may change in the future without warning (so be cautious but don't depend on it).
    Anything held in the task object itself (e.g. stack allocations, handles to the relevant process/thread, etc.) will be dropped as normal
        at a non-deterministic time in the near future (usually when another terminate call occurs in the scheduler, or sooner if i add a cleanup task that periodically cleans up terminated tasks). */
#[inline]
pub fn terminate_current_task() -> ! {
    yield_to_scheduler(SchedulerCommand::Terminate);
    unreachable!();  // the scheduler will drop the task when yield is called with the Terminate command
}
/* Shorthand for yielding as part of a spinloop. */
#[inline]
pub fn spin_yield(){
    yield_to_scheduler(SchedulerCommand::PushBack);
}

/* The scheduler command given to _cs_push is then passed over to the scheduler.
    It is used to tell the scheduler what to do with the task that just finished. */
pub enum SchedulerCommand<'a> {
    /// Push the current task back to the run_queue, and run it again once it's time
    PushBack,
    /// Discard the current task - it has terminated. (this does not perform unwinding).
    /// It is preferred to use terminate_current_task or similar instead of yield_to_scheduler(Terminate) where possible.
    Terminate,
    /// Sleep for the requested number of PIT ticks
    SleepNTicks(usize),
    /// Push a waiting list entry to the given waiting list, then unlock the mutex by dropping the guard
    /// (the Option<> is used internally, and must always be passed as Some(). Passing a None may (will) cause a kernel panic.
    PushToWaitingList(core::cell::Cell<Option<YMutexGuard<'a,VecDeque<WaitingListEntry>>>>),
}

/* Do not call this function yourself! (unless you know what you're doing). Use yield_to_scheduler instead!
    This function happens after the previous context is saved, but before the next one is loaded. It's this function's job to determine what to run next (and then run it). */
#[inline]
pub(super) fn schedule(command: SchedulerCommand, rsp: StackPointer) -> ! {
    if super::interruptions::is_sched_yield_disabled() { panic!("schedule() called when interruptions were disabled?"); }
    _IS_EXECUTING_TASK.get().store(false, Ordering::Release);
    let mut current_task = _CURRENT_TASK.lock().take().expect("schedule() called but no task currently active?");
    {
        let mut state = _SCHEDULER_STATE.lock();
        // Update current task
        current_task.set_rsp(rsp);
        
        // FIXME: DO NOT allocate/deallocate phys/virt memory while inside the scheduler!
        // PagingContexts and the physical memory allocator both use WLocks!!
        
        // Push previous task back onto the run queue
        match &command {
            SchedulerCommand::Terminate => {
                // Terminate the task
                klog!(Debug, SCHEDULER, "Terminating task: {}", current_task.task_id);
                state.deferred_drop.push(current_task)
            }
            
            SchedulerCommand::SleepNTicks(ticks) => {
                // Sleep
                let wake_at = get_scheduler_ticks() + ticks;
                klog!(Debug, SCHEDULER, "Task {} sleeping for {} ticks. (until t={})", current_task.task_id, ticks, wake_at);
                state.sleeping.push_back((current_task, wake_at));
            }
            
            SchedulerCommand::PushToWaitingList(list) => {
                // Construct and push a waiting list entry
                let mut list = list.replace(None).expect("Schedule was passed PushToWaitingList(None)!");
                klog!(Debug, SCHEDULER, "Task {} waiting on list.");
                list.push_back(WaitingListEntry { cpu: super::get_cpu_num(), task: current_task });
            }
            
            _ => {
                // Push back onto run queue
                klog!(Debug, SCHEDULER, "Suspending task: {}", current_task.task_id);
                state.run_queue.push_back(current_task);
            }
        }
    };  // <-- lock is released here
    
    // Pick the next task off of the run queue
    loop {
        let next_task = {
            let mut state = _SCHEDULER_STATE.lock();
            (match &command {
                _ => {
                    // Take next task
                    state.run_queue.pop_front()
                }
            }).map(|task|(task, state))
        };
        
        if let Some((next_task, state_guard)) = next_task {
            klog!(Debug, SCHEDULER, "Resuming task: {}", next_task.task_id);
            let ni = disable_interruptions(); drop(state_guard);  // (unlock state but keep interruptions disabled)
            resume_context(next_task, ni)
        } else {
            // No tasks to do
            // spin for now
            core::hint::spin_loop();
        }
    }
}

/* Resume the requested task, discarding the current one (if any). */
#[inline]
pub fn resume_context(task: Task, state_guard: KMutexGuard<SchedulerState>) -> !{
    // Note: state_guard contains a no-interruptions guard within it, ensuring we're not interrupted
    let rsp = task.get_rsp();
    
    // resume task
    // Note: setting the current_task is handled by __resume_callback
    unsafe { cswitch_impl::resume_context(rsp, (task, state_guard)) };
}
/* Called by _cs_pop after the stack has been switched to the task's stack */
#[inline]
pub(super) fn __resume_callback(args: (Task,NoInterruptionsGuard)){
    let (task, ni) = args;
    
    // set active task
    _CURRENT_TASK.lock().insert(task);
    _IS_EXECUTING_TASK.store(true, Ordering::Release);
    
    // TODO: Switch paging context if necessary?
    
    // Done! We are now "in" the task, with CURRENT_TASK set and the stack switched, so we can now enable interruptions without issue
    drop(ni);
}

// NOTE: Anything done while _SCHEDULER_STATE is locked will be done with interruptions_disabled (because of how KMutexes work)
/* Initialise the scheduler for the current CPU, before creating a kernel task to represent the current stack.
    Once this has been called, it is ok to call yield_to_scheduler.
    (calling this again will discard a large amount of the scheduler's state for the current CPU, so uh, don't)*/
pub fn init_scheduler(stack: Option<alloc::boxed::Box<dyn crate::memory::alloc_util::AnyAllocatedStack>>){
    let boot_task = {
        let mut state = _SCHEDULER_STATE.lock();
        // initialise run queue?
        state.run_queue.reserve(8);
        
        // Initialise task
        // Note: resuming the task is undefined (however that is the same for all "currently active tasks" - as they must be paused first)
        let task = unsafe { Task::new_with_rsp(TaskType::KernelTask, core::ptr::null_mut(), stack) };
        let task_id = task.task_id;
        
        // All gucci :)
        // log message
        klog!(Info, SCHEDULER, "Initialised scheduler on CPU {}. Bootstrapper task has become task {}.", super::get_cpu_num(), task_id);
        
        // Signal that scheduler is online
        BSP_SCHEDULER_READY.store(true,core::sync::atomic::Ordering::Release);
        // Return the task
        task
    };
    
    // Set current task
    // We should not be holding any locks once we initialise the current task to a non-None value,
    // as otherwise any unexpected event (or held lock) would attempt to yield to the scheduler
    // (which cannot be done if the scheduler lock is held, causing what I think is a stack overflow)
    _CURRENT_TASK.insert(boot_task);
}
/// If true, then the scheduler has been initialised on the bootstrap processor
static BSP_SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

/* Push a new task to the current scheduler's run queue. */
pub fn push_task(task: Task){
    klog!(Debug, SCHEDULER, "Pushing new task: {}", task.task_id);
    _SCHEDULER_STATE.lock().run_queue.push_back(task);
}
/* Push a new task to another scheduler's run queue. */
pub fn push_task_to(cpu: usize, task: Task){
    klog!(Debug, SCHEDULER, "Pushing new task to CPU {}: {}", cpu, task.task_id);
    _SCHEDULER_STATE.0.get_for(cpu).lock().run_queue.push_back(task);
}

/* Advances the scheduler's clock by 1 tick. Called by the PIT. */
pub fn _scheduler_tick(){
    {
        let mut state = _SCHEDULER_STATE.lock();
        let current_ticks = _SCHEDULER_TICKS.fetch_add(1, Ordering::SeqCst)+1;
        
        // wake tasks sleeping on clock ticks
        // We go in reverse order to ensure that the indexes still line up 
        for i in (0..state.sleeping.len()).rev() {
            let (_, wake_at) = state.sleeping[i];
            if wake_at <= current_ticks {
                // Take item
                // Since this is either the back, or we've already processed the back, we can safely perform a swap_remove_back
                let (task, _) = state.sleeping.swap_remove_back(i).unwrap();
                klog!(Debug, SCHEDULER, "Waking task: {}", task.task_id);
                // Push to run queue
                state.run_queue.push_back(task);
            }
        }
    }
}

/* Returns true if the scheduler is currently executing a task. Returns false otherwise (i.e. it's instead executing bootstrap or scheduler code). */
#[inline(always)]
pub fn is_executing_task() -> bool {
    _IS_EXECUTING_TASK.load(Ordering::Relaxed) && _CURRENT_TASK.lock().is_some()
}
/* Get the ID of the current task, or None if the scheduler is running right now instead of a specific task. */
#[inline(always)]
pub fn get_executing_task_id() -> Option<usize> {
    _CURRENT_TASK.lock().as_ref().map(|t|t.task_id)
}
/* Get the current tick count on the current CPU's scheduler.
This may differ between CPUs, and is not a good way of keeping time, but is lowlevel and does not rely on the RTC or anything complicated like that. 
Will probably be deprecated once support for actual time is added. */
#[inline(always)]
pub fn get_scheduler_ticks() -> usize {
    _SCHEDULER_TICKS.load(Ordering::Relaxed)
}