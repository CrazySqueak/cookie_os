
pub struct SchedulerYield;
impl spin::relax::RelaxStrategy for SchedulerYield {
    fn relax(){
        todo!()  // scheduler yield
    }
}
pub type YieldSpin = super::spinlocks::SpinLockStrategy<SchedulerYield>;

pub type YMutex<T> = super::spinlocks::BaseSpinMutex<T,YieldSpin>;
pub type YMutexGuard<'a,T> = super::spinlocks::BaseSpinMutexGuard<'a,T,YieldSpin>;
pub type MappedYMutexGuard<'a,T> = super::spinlocks::MappedBaseSpinMutexGuard<'a,T,YieldSpin>;

pub type YRwLock<T> = super::spinlocks::BaseSpinRwLock<T,YieldSpin>;
pub type YRwLockReadGuard<'a,T> = super::spinlocks::BaseSpinRwLockReadGuard<'a,T,YieldSpin>;
pub type YRwLockWriteGuard<'a,T> = super::spinlocks::BaseSpinRwLockWriteGuard<'a,T,YieldSpin>;
pub type YRwLockUpgradableGuard<'a,T> = super::spinlocks::BaseSpinRwLockUpgradableGuard<'a,T,YieldSpin>;
pub type MappedYRwLockReadGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockReadGuard<'a,T,YieldSpin>;
pub type MappedYRwLockWriteGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockWriteGuard<'a,T,YieldSpin>;
