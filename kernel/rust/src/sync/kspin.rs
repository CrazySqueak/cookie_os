
use super::spinlocks::SpinLockStrategy;
pub type BlockingSpin = SpinLockStrategy<super::llspin::BlockingSpin>;

pub type KMutex<T> = super::nointerruptionslocks::BaseNoInterruptionsMutex<T,BlockingSpin>;
pub type KMutexGuard<'a,T> = super::nointerruptionslocks::BaseNoInterruptionsMutexGuard<'a,T,BlockingSpin>;
pub type MappedKMutexGuard<'a,T> = super::nointerruptionslocks::MappedBaseNoInterruptionsMutexGuard<'a,T,BlockingSpin>;

pub type KRwLock<T> = super::nointerruptionslocks::BaseNoInterruptionsRwLock<T,BlockingSpin>;
pub type KRwLockReadGuard<'a,T> = super::nointerruptionslocks::BaseNoInterruptionsRwLockReadGuard<'a,T,BlockingSpin>;
pub type KRwLockWriteGuard<'a,T> = super::nointerruptionslocks::BaseNoInterruptionsRwLockWriteGuard<'a,T,BlockingSpin>;
pub type KRwLockUpgradableGuard<'a,T> = super::nointerruptionslocks::BaseNoInterruptionsRwLockUpgradableGuard<'a,T,BlockingSpin>;
pub type MappedKRwLockReadGuard<'a,T> = super::nointerruptionslocks::MappedBaseNoInterruptionsRwLockReadGuard<'a,T,BlockingSpin>;
pub type MappedKRwLockWriteGuard<'a,T> = super::nointerruptionslocks::MappedBaseNoInterruptionsRwLockWriteGuard<'a,T,BlockingSpin>;
