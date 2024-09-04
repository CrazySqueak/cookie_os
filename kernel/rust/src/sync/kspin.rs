//! Kernel Spin
//! 
//! K-locks are primitive spinlocks that loop indefinitely until the required condition is met.
//! It is recommended that you DO NOT YIELD the current task while you hold a K-lock, as doing so may deadlock the kernel.
use super::spin::*;
use core::hint::spin_loop;

pub struct CpuSpin;
impl Relax for CpuSpin {
    fn relax(){
        spin_loop()
    }
}

pub type KMutexRaw = SpinMutexRaw<CpuSpin>;
pub type KMutex<T> = SpinMutex<T,CpuSpin>;
pub type KMutexGuard<'a,T> = SpinMutexGuard<'a,T,CpuSpin>;
pub type MappedKMutexGuard<'a,T> = MappedSpinMutexGuard<'a,T,CpuSpin>;
pub type KRwLockRaw = SpinRwLockRaw<CpuSpin>;
pub type KRwLock<T> = SpinRwLock<T,CpuSpin>;
pub type KRwLockReadGuard<'a,T> = SpinRwLockReadGuard<'a,T,CpuSpin>;
pub type KRwLockWriteGuard<'a,T> = SpinRwLockWriteGuard<'a,T,CpuSpin>;
pub type MappedKRwLockReadGuard<'a,T> = MappedSpinRwLockReadGuard<'a,T,CpuSpin>;
pub type MappedKRwLockWriteGuard<'a,T> = MappedSpinRwLockWriteGuard<'a,T,CpuSpin>;