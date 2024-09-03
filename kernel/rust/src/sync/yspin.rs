//! Yield Spin
//! 
//! Y-locks are primitive spinlocks that yield to the scheduler indefinitely until the required condition is met.
use super::spin::*;
use core::hint::spin_loop;
use crate::multitasking::scheduler;

pub struct SchedulerYield;
impl Relax for SchedulerYield {
    fn relax(){
        scheduler::yield_to_scheduler(scheduler::SchedulerCommand::PushBack)
    }
}

pub type YMutex<T> = SpinMutex<T,SchedulerYield>;
pub type YMutexGuard<'a,T> = SpinMutexGuard<'a,T,SchedulerYield>;
pub type MappedYMutexGuard<'a,T> = MappedSpinMutexGuard<'a,T,SchedulerYield>;
pub type YRwLock<T> = SpinRwLock<T,SchedulerYield>;
pub type YRwLockReadGuard<'a,T> = SpinRwLockReadGuard<'a,T,SchedulerYield>;
pub type YRwLockWriteGuard<'a,T> = SpinRwLockWriteGuard<'a,T,SchedulerYield>;
pub type MappedYRwLockReadGuard<'a,T> = MappedSpinRwLockReadGuard<'a,T,SchedulerYield>;
pub type MappedYRwLockWriteGuard<'a,T> = MappedSpinRwLockWriteGuard<'a,T,SchedulerYield>;