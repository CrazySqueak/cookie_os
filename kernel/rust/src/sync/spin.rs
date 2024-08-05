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

pub type Mutex<T> = spin::mutex::Mutex<T, SchedulerYield>;
pub type MutexGuard<'a, T> = spin::mutex::MutexGuard<'a, T>;

pub type RwLock<T> = spin::rwlock::RwLock<T, SchedulerYield>;
pub type RwLockReadGuard<'a, T> = spin::rwlock::RwLockReadGuard<'a, T>;
pub type RwLockWriteGuard<'a, T> = spin::rwlock::RwLockWriteGuard<'a, T, SchedulerYield>;
pub type RwLockUpgradableGuard<'a, T> = spin::rwlock::RwLockUpgradableGuard<'a, T, SchedulerYield>;