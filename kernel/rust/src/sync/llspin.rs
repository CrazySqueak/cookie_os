
pub type BlockingSpin = super::spinlocks::SpinLockStrategy<spin::relax::Spin>;

pub type LLMutex<T> = super::spinlocks::BaseSpinMutex<T,BlockingSpin>;
pub type LLMutexGuard<'a,T> = super::spinlocks::BaseSpinMutexGuard<'a,T,BlockingSpin>;
pub type MappedLLMutexGuard<'a,T> = super::spinlocks::MappedBaseSpinMutexGuard<'a,T,BlockingSpin>;

pub type LLRwLock<T> = super::spinlocks::BaseSpinRwLock<T,BlockingSpin>;
pub type LLRwLockReadGuard<'a,T> = super::spinlocks::BaseSpinRwLockReadGuard<'a,T,BlockingSpin>;
pub type LLRwLockWriteGuard<'a,T> = super::spinlocks::BaseSpinRwLockWriteGuard<'a,T,BlockingSpin>;
pub type LLRwLockUpgradableGuard<'a,T> = super::spinlocks::BaseSpinRwLockUpgradableGuard<'a,T,BlockingSpin>;
pub type MappedLLRwLockReadGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockReadGuard<'a,T,BlockingSpin>;
pub type MappedLLRwLockWriteGuard<'a,T> = super::spinlocks::MappedBaseSpinRwLockWriteGuard<'a,T,BlockingSpin>;
