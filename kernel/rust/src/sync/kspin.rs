//! Kernel Spin
//! 
//! K-locks are primitive spinlocks that loop indefinitely until the required condition is met.

use lock_api::{RawMutex,RawRwLock,GuardSend};
use core::sync::atomic::*;
use core::hint::spin_loop;

pub struct KMutexRaw(AtomicBool);
unsafe impl RawMutex for KMutexRaw {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicBool::new(false));
    
    fn try_lock(&self) -> bool {
        self.0.compare_exchange(false,true, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    fn lock(&self) {
        while self.0.compare_exchange_weak(false,true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.0.load(Ordering::Relaxed) { spin_loop(); }
        }
    }
    
    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
    }
}
pub type KMutex<T> = lock_api::Mutex<KMutexRaw,T>;
pub type KMutexGuard<'a,T> = lock_api::MutexGuard<'a,KMutexRaw,T>;
pub type MappedKMutexGuard<'a,T> = lock_api::MappedMutexGuard<'a,KMutexRaw,T>;

const WRITER: usize = 1<<63;
const UPGRADER: usize = 1<<62;
const EXCLUSIVE_THRESHOLD: usize = 1<<61;
pub struct KRwLockRaw(AtomicUsize);
unsafe impl RawRwLock for KRwLockRaw {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicUsize::new(0));
    
    fn lock_shared(&self) {
        while !self.try_lock_shared() {
            // Failed
            while self.0.load(Ordering::Relaxed)>=EXCLUSIVE_THRESHOLD { spin_loop(); }
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
            while self.0.load(Ordering::Relaxed)!=0 { spin_loop(); }
        }
    }
    
    fn try_lock_exclusive(&self) -> bool {
        self.0.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    
    unsafe fn unlock_exclusive(&self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
    }
}
unsafe impl lock_api::RawRwLockDowngrade for KRwLockRaw {
    unsafe fn downgrade(&self) {
        // Subtracting (WRITER-1) means that when x=WRITER, x will now equal 1 (a single shared lock), or so on
        self.0.fetch_sub(WRITER-1, Ordering::Release);
    }
}
pub type KRwLock<T> = lock_api::RwLock<KRwLockRaw,T>;
pub type KRwLockReadGuard<'a,T> = lock_api::RwLockReadGuard<'a,KRwLockRaw,T>;
pub type KRwLockWriteGuard<'a,T> = lock_api::RwLockWriteGuard<'a,KRwLockRaw,T>;
pub type MappedKRwLockReadGuard<'a,T> = lock_api::MappedRwLockReadGuard<'a,KRwLockRaw,T>;
pub type MappedKRwLockWriteGuard<'a,T> = lock_api::MappedRwLockWriteGuard<'a,KRwLockRaw,T>;