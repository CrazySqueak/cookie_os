//! WaitLocks
//!
//! WaitLocks are kernel locking primitives based on the "Waiting List"

use lock_api::{RawMutex,RawRwLock,GuardSend};
use core::sync::atomic::*;
use super::WaitingList;

pub struct WMutexRaw(AtomicBool,WaitingList);
unsafe impl RawMutex for WMutexRaw {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicBool::new(false), WaitingList::new());
    
    fn try_lock(&self) -> bool {
        self.0.compare_exchange(false,true, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    fn lock(&self) {
        self.1.wait_until(||self.try_lock());
    }
    
    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
        self.1.notify_one();
    }
}
pub type WMutex<T> = lock_api::Mutex<WMutexRaw,T>;
pub type WMutexGuard<'a,T> = lock_api::MutexGuard<'a,WMutexRaw,T>;
pub type MappedWMutexGuard<'a,T> = lock_api::MappedMutexGuard<'a,WMutexRaw,T>;

const WRITER: usize = 1<<63;
const UPGRADER: usize = 1<<62;
const EXCLUSIVE_THRESHOLD: usize = 1<<61;
// (lock, wait_shared, wait_exclusive)
pub struct WRwLockRaw(AtomicUsize,WaitingList,WaitingList);
unsafe impl RawRwLock for WRwLockRaw {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicUsize::new(0),WaitingList::new(),WaitingList::new());
    
    fn lock_shared(&self) {
        self.1.wait_until(||self.try_lock_shared())
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
        let oldreaders = self.0.fetch_sub(1, Ordering::Release);
        // If no more readers hold the lock, notify one writer or all readers.
        if oldreaders == 1 { if !self.2.notify_one() {self.1.notify_all();} }
    }
    
    fn lock_exclusive(&self) {
        self.2.wait_until(||self.try_lock_exclusive())
    }
    
    fn try_lock_exclusive(&self) -> bool {
        self.0.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    
    unsafe fn unlock_exclusive(&self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
        // Notify one writer or all readers.
        if !self.2.notify_one() {self.1.notify_all();}
    }
}
unsafe impl lock_api::RawRwLockDowngrade for WRwLockRaw {
    unsafe fn downgrade(&self) {
        // Subtracting (WRITER-1) means that when x=WRITER, x will now equal 1 (a single shared lock), or so on
        self.0.fetch_sub(WRITER-1, Ordering::Release);
        // Notify readers (but not writers as we still hold the lock (thus writers can't acquire it but readers can))
        self.1.notify_all();
    }
}
pub type WRwLock<T> = lock_api::RwLock<WRwLockRaw,T>;
pub type WRwLockReadGuard<'a,T> = lock_api::RwLockReadGuard<'a,WRwLockRaw,T>;
pub type WRwLockWriteGuard<'a,T> = lock_api::RwLockWriteGuard<'a,WRwLockRaw,T>;
pub type MappedWRwLockReadGuard<'a,T> = lock_api::MappedRwLockReadGuard<'a,WRwLockRaw,T>;
pub type MappedWRwLockWriteGuard<'a,T> = lock_api::MappedRwLockWriteGuard<'a,WRwLockRaw,T>;