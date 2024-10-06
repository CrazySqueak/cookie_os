
use crate::multitasking::scheduler::spin_yield;

pub struct SchedulerYield;
impl spin::relax::RelaxStrategy for SchedulerYield {
    #[inline]
    fn relax(){
        spin_yield()
    }
}
pub type YieldSpin = SchedulerYield;

pub type YMutex<T> = super::spinlocks::BaseSpinMutex<T,YieldSpin>;
pub type YMutexGuard<'a,T> = super::spinlocks::BaseSpinMutexGuard<'a,T,YieldSpin>;
pub type MappedYMutexGuard<'a,T> = super::spinlocks::MappedBaseSpinMutexGuard<'a,T,YieldSpin>;
pub type ArcYMutexGuard<T> = super::spinlocks::ArcBaseSpinMutexGuard<T,YieldSpin>;

pub type YRwLock<T> = super::spinlocks::BaseSpinRwLock<T,YieldSpin>;
pub type YRwLockReadGuard<'a,T> = super::spinlocks::BaseSpinRwLockReadGuard<'a,T,YieldSpin>;
pub type YRwLockWriteGuard<'a,T> = super::spinlocks::BaseSpinRwLockWriteGuard<'a,T,YieldSpin>;
pub type YRwLockUpgradableGuard<'a,T> = super::spinlocks::BaseSpinRwLockUpgradableGuard<'a,T,YieldSpin>;
pub type MappedYRwLockReadGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockReadGuard<'a,T,YieldSpin>;
pub type MappedYRwLockWriteGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockWriteGuard<'a,T,YieldSpin>;
pub type ArcYRwLockReadGuard<T> = super::spinlocks::ArcBaseSpinRwLockReadGuard<T,YieldSpin>;
pub type ArcYRwLockWriteGuard<T> = super::spinlocks::ArcBaseSpinRwLockWriteGuard<T,YieldSpin>;
pub type ArcYRwLockUpgradableGuard<T> = super::spinlocks::ArcBaseSpinRwLockUpgradableGuard<T,YieldSpin>;
