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

const WRITER: usize = 1<<50;
const UPGRADER: usize = 1<<49;
const EXCLUSIVE_THRESHOLD: usize = 1<<48;
// (lock, wait_shared, wait_exclusive)
pub struct WRwLockRaw(AtomicUsize,WaitingList,WaitingList);
impl WRwLockRaw {
    /* If locked exclusively, returns Err(). Otherwise returns Ok(x) */
    pub fn reader_count(&self) -> Result<usize,usize> {
        let result = self.0.load(Ordering::Relaxed);
        if result >= EXCLUSIVE_THRESHOLD { return Err(result); }
        Ok(result)
    }
}
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
        self.0.fetch_sub(1, Ordering::Release);
        // Notify one writer + all readers. (we might not be the only reader if an upgrader exists, and they need to be notified anyway)
        // (we notify both because there's no guarantee that the lock will be unlocked shared when the writer tries to take it, which could cause the readers to be stuck in limbo as a result)
        self.2.notify_one();
        self.1.notify_all();
    }
    
    fn lock_exclusive(&self) {
        self.2.wait_until(||self.try_lock_exclusive())
    }
    
    fn try_lock_exclusive(&self) -> bool {
        self.0.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    
    unsafe fn unlock_exclusive(&self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
        // Notify one writer + all readers.
        self.2.notify_one();
        self.1.notify_all();
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
unsafe impl lock_api::RawRwLockUpgrade for WRwLockRaw {
    fn lock_upgradable(&self) {
        self.2.wait_until(||self.try_lock_upgradable())
    }
    fn try_lock_upgradable(&self) -> bool {
        let value = self.0.fetch_add(UPGRADER, Ordering::Acquire);
        if value>=EXCLUSIVE_THRESHOLD {  // (existing writer or upgrader)
            self.0.fetch_sub(UPGRADER, Ordering::Release);
            return false;
        }
        return true;
    }
    
    unsafe fn unlock_upgradable(&self) {
        let old = self.0.fetch_sub(UPGRADER,Ordering::Release);
        // Wake one writer (if 0) + all readers
        if old == UPGRADER { self.2.notify_one(); }
        self.1.notify_all();
    }
    unsafe fn upgrade(&self) {
        self.2.wait_until(||unsafe{self.try_upgrade()})
    }
    unsafe fn try_upgrade(&self) -> bool {
        self.0.compare_exchange(UPGRADER, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
}
unsafe impl lock_api::RawRwLockUpgradeDowngrade for WRwLockRaw {
    unsafe fn downgrade_upgradable(&self) {
        self.0.fetch_sub(UPGRADER-1,Ordering::Release);
    }
    unsafe fn downgrade_to_upgradable(&self) {
        self.0.fetch_sub(WRITER-UPGRADER,Ordering::Release);
    }
}
pub type WRwLock<T> = lock_api::RwLock<WRwLockRaw,T>;
pub type WRwLockReadGuard<'a,T> = lock_api::RwLockReadGuard<'a,WRwLockRaw,T>;
pub type WRwLockWriteGuard<'a,T> = lock_api::RwLockWriteGuard<'a,WRwLockRaw,T>;
pub type WRwLockUpgradableGuard<'a,T> = lock_api::RwLockUpgradableReadGuard<'a,WRwLockRaw,T>;
pub type MappedWRwLockReadGuard<'a,T> = lock_api::MappedRwLockReadGuard<'a,WRwLockRaw,T>;
pub type MappedWRwLockWriteGuard<'a,T> = lock_api::MappedRwLockWriteGuard<'a,WRwLockRaw,T>;
