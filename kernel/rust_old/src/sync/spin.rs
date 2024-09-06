//! Primitive spinlocks

use lock_api::{RawMutex,RawRwLock,GuardSend};
use core::sync::atomic::*;

pub trait Relax {
    fn relax();
}

pub struct SpinMutexRaw<R:Relax>(AtomicBool,core::marker::PhantomData<R>);
unsafe impl<R:Relax> RawMutex for SpinMutexRaw<R> {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicBool::new(false),core::marker::PhantomData);
    
    fn try_lock(&self) -> bool {
        self.0.compare_exchange(false,true, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    fn lock(&self) {
        while self.0.compare_exchange_weak(false,true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.0.load(Ordering::Relaxed) { R::relax() }
        }
    }
    
    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}
pub type SpinMutex<T,R> = lock_api::Mutex<SpinMutexRaw<R>,T>;
pub type SpinMutexGuard<'a,T,R> = lock_api::MutexGuard<'a,SpinMutexRaw<R>,T>;
pub type MappedSpinMutexGuard<'a,T,R> = lock_api::MappedMutexGuard<'a,SpinMutexRaw<R>,T>;

const WRITER: usize = 1<<63;
const UPGRADER: usize = 1<<62;
const EXCLUSIVE_THRESHOLD: usize = 1<<61;
pub struct SpinRwLockRaw<R:Relax>(AtomicUsize,core::marker::PhantomData<R>);
unsafe impl<R:Relax> RawRwLock for SpinRwLockRaw<R> {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicUsize::new(0),core::marker::PhantomData);
    
    fn lock_shared(&self) {
        while !self.try_lock_shared() {
            // Failed
            while self.0.load(Ordering::Relaxed)>=EXCLUSIVE_THRESHOLD { R::relax(); }
        }
    }
    
    fn try_lock_shared(&self) -> bool {
        let value = self.0.fetch_add(1, Ordering::Acquire);
        if value>=EXCLUSIVE_THRESHOLD {
            self.0.fetch_sub(1, Ordering::Release);
            return false;
        }
        return true;
    }
    
    unsafe fn unlock_shared(&self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
    
    fn lock_exclusive(&self) {
        while self.0.compare_exchange_weak(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.0.load(Ordering::Relaxed)!=0 { R::relax(); }
        }
    }
    
    fn try_lock_exclusive(&self) -> bool {
        self.0.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    
    unsafe fn unlock_exclusive(&self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
    }
}
unsafe impl<R:Relax> lock_api::RawRwLockDowngrade for SpinRwLockRaw<R> {
    unsafe fn downgrade(&self) {
        // Subtracting (WRITER-1) means that when x=WRITER, x will now equal 1 (a single shared lock), or so on
        self.0.fetch_sub(WRITER-1, Ordering::Release);
    }
}
pub type SpinRwLock<T,R> = lock_api::RwLock<SpinRwLockRaw<R>,T>;
pub type SpinRwLockReadGuard<'a,T,R> = lock_api::RwLockReadGuard<'a,SpinRwLockRaw<R>,T>;
pub type SpinRwLockWriteGuard<'a,T,R> = lock_api::RwLockWriteGuard<'a,SpinRwLockRaw<R>,T>;
pub type MappedSpinRwLockReadGuard<'a,T,R> = lock_api::MappedRwLockReadGuard<'a,SpinRwLockRaw<R>,T>;
pub type MappedSpinRwLockWriteGuard<'a,T,R> = lock_api::MappedRwLockWriteGuard<'a,SpinRwLockRaw<R>,T>;