use super::{KMutex};
use alloc::collections::VecDeque;
use crate::multitasking::scheduler;
use crate::multitasking::without_interruptions;

pub struct WaitingListEntry {
    task: crate::multitasking::Task,
    cpu: usize,
}

/// A scheduler-based waiting list
/// Tasks here will sleep until woken by a corresponding notify() call.
pub struct WaitingList(KMutex<VecDeque<WaitingListEntry>>);
impl WaitingList {
    pub const fn new() -> Self {
        Self(KMutex::new(VecDeque::new()))
    }
    
    /// Yield to the scheduler, and wait until the thread is notified
    /// Note: This makes no guarantee that a notify hasn't happened in between you checking the predicate and calling wait()
    ///       For more robust behaviour, consider using wait_ifnt or wait_until instead.
    pub fn wait(&self) {
        let list = self.0.lock();
        // The scheduler takes ownership of the lock and drops it after pushing
        scheduler::yield_to_scheduler(scheduler::SchedulerCommand::PushToWaitingList(core::cell::Cell::new(Some(list))));
    }
    /// Lock the waiting list, then check the predicate, and then finally wait *iff* the predicate was false
    /// This method guarantees that notify() has not been called between checking the predicate and suspending the thread
    /// Returns true if the thread was suspended, false if the predicate returned true early.
    pub fn wait_ifnt(&self, predicate: impl FnOnce()->bool) -> bool {
        let list = self.0.lock();
        if predicate() { return false; }  // Predicate returned true, so return early
        // The scheduler takes ownership of the lock and drops it after pushing
        scheduler::yield_to_scheduler(scheduler::SchedulerCommand::PushToWaitingList(core::cell::Cell::new(Some(list))));
        true
    }
    /// A version of wait_ifnt that checks the predicate again after resuming, and keeps suspending until the predicate returns true
    pub fn wait_until(&self, predicate: impl Fn()->bool + Copy) {
        // Keep trying until the predicate returns true (causing wait_ifnt to return false)
        while self.wait_ifnt(predicate){}
    }
    
    fn notify_inner(&self, list: &mut super::KMutexGuard<'_,VecDeque<WaitingListEntry>>) -> bool {
        match list.pop_front() {
            Some(entry) => {
                scheduler::push_task_to(entry.cpu, entry.task);
                true
            },
            None => { false },
        }
    }
    /// Wake up one thread waiting on this list
    /// Returns true if one was waiting, false otherwise
    pub fn notify_one(&self) -> bool { without_interruptions(||{
        let mut list = self.0.lock();
        self.notify_inner(&mut list)
    })}
    /// Wake up all threads waiting on this list
    pub fn notify_all(&self) { without_interruptions(||{
        let mut list = self.0.lock();
        // As notify_inner returns true on each success, we can just do this.
        while self.notify_inner(&mut list){}
    })}
}