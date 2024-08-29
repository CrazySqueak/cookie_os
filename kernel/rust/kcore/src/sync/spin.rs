use spin::relax::{RelaxStrategy,Spin};
//use crate::multitasking::{is_bsp_scheduler_initialised,is_executing_task,yield_to_scheduler,SchedulerCommand};
use crate::forward::scheduler;

pub struct SchedulerYield;
impl RelaxStrategy for SchedulerYield {
    #[inline(always)]
    fn relax(){
        if true{//is_bsp_scheduler_initialised() {  // TODO
            if scheduler::is_executing_task() {
                // Yield
                scheduler::spin_yield()
            } else {
                // Our scheduler is not executing a task, so we cannot yield to it - this means we are probably the scheduler
                // it is likely that another CPU is accessing this resource and we simply have to spin and hope it unblocks (because the scheduler can't continue without it)
                // TODO: Add monotonic time measurement somehow and panic if 1+ seconds pass (either a deadlock or a hard-freeze - neither are good signs)
                Spin::relax();
            }
        } else {
            panic!("Waiting for lock but scheduler not configured yet! Possible deadlock?");
        }
    }
}

pub type Mutex<T, R=SchedulerYield> = spin::mutex::Mutex<T, R>;
pub type MutexGuard<'a, T> = spin::mutex::MutexGuard<'a, T>;

pub type RwLock<T, R=SchedulerYield> = spin::rwlock::RwLock<T, R>;
pub type RwLockReadGuard<'a, T> = spin::rwlock::RwLockReadGuard<'a, T>;
pub type RwLockWriteGuard<'a, T, R=SchedulerYield> = spin::rwlock::RwLockWriteGuard<'a, T, R>;
pub type RwLockUpgradableGuard<'a, T, R=SchedulerYield> = spin::rwlock::RwLockUpgradableGuard<'a, T, R>;