use spin::relax::{RelaxStrategy,Spin};
use crate::scheduler::{is_bsp_scheduler_initialised,is_scheduler_ready,yield_to_scheduler,SchedulerCommand};

pub struct SchedulerYield;
impl RelaxStrategy for SchedulerYield {
    #[inline(always)]
    fn relax(){
        if is_bsp_scheduler_initialised() {
            if is_scheduler_ready() {
                // Yield
                yield_to_scheduler(SchedulerCommand::PushBack)
            } else {
                // Our scheduler is not yet ready, so this CPU is either trampolining or running scheduler code (and contending with other CPUs in the process)
                Spin::relax();
            }
        } else {
            panic!("Waiting for lock but scheduler not configured yet! Possible deadlock?");
        }
    }
}

pub struct AlwaysPanic;
// Used for Mutexes in certain cpu-local, scheduler-critical environments, where a lock being held prevents the scheduler from proceeding (and thus guarantees a deadlock)
impl RelaxStrategy for AlwaysPanic {
    #[inline(always)]
    fn relax(){
        panic!("Waiting for lock, but item is cpu-local and critical to scheduler functionality! Deadlock?");
    }
}

pub type Mutex<T, R=SchedulerYield> = spin::mutex::Mutex<T, R>;
pub type MutexGuard<'a, T> = spin::mutex::MutexGuard<'a, T>;

pub type RwLock<T, R=SchedulerYield> = spin::rwlock::RwLock<T, R>;
pub type RwLockReadGuard<'a, T> = spin::rwlock::RwLockReadGuard<'a, T>;
pub type RwLockWriteGuard<'a, T, R=SchedulerYield> = spin::rwlock::RwLockWriteGuard<'a, T, R>;
pub type RwLockUpgradableGuard<'a, T, R=SchedulerYield> = spin::rwlock::RwLockUpgradableGuard<'a, T, R>;