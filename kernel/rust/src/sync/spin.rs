use spin::relax::RelaxStrategy;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

static SCHEDULER_READY: AtomicBool = AtomicBool::new(false);

pub struct SchedulerYield;
impl RelaxStrategy for SchedulerYield {
    #[inline(always)]
    fn relax(){
        if SCHEDULER_READY.load(Ordering::SeqCst) {
            todo!();  // yield
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