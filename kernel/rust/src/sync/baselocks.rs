
use lock_api::{RawMutex,RawRwLock,GuardSend};
use core::sync::atomic::*;

pub trait MutexStrategy {
    const INIT: Self;
    /// Called after the mutex has unlocked
    fn on_unlock(&self);
    /// Called when an attempt to lock the mutex is made, but fails
    fn lock_relax(&self);
}

pub struct BaseMutexRaw<S:MutexStrategy>(AtomicBool,S);
impl<S:MutexStrategy> BaseMutexRaw<S> {
    pub fn is_locked(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
unsafe impl<S:MutexStrategy> RawMutex for BaseMutexRaw<S> {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicBool::new(false),S::INIT);
    
    fn try_lock(&self) -> bool {
        let ok = self.0.compare_exchange(false,true, Ordering::Acquire, Ordering::Relaxed).is_ok();
        ok
    }
    fn lock(&self) {
        while self.0.compare_exchange_weak(false,true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.0.load(Ordering::Relaxed) { self.1.lock_relax() }
        }
    }
    
    unsafe fn unlock(&self) {
        self.0.store(false, Ordering::Release);
        self.1.on_unlock();
    }
}
pub type BaseMutex<T,S> = lock_api::Mutex<BaseMutexRaw<S>,T>;
pub type BaseMutexGuard<'a,T,S> = lock_api::MutexGuard<'a,BaseMutexRaw<S>,T>;
pub type MappedBaseMutexGuard<'a,T,S> = lock_api::MappedMutexGuard<'a,BaseMutexRaw<S>,T>;


pub trait RwLockStrategy {
    const INIT: Self;
    
    /// Called after the rwlock has unlocked from reading
    /// (this and the other _unlock methods that take a reader count may be called spuriously - whenever the reader count is decremented e.g. if reading failed)
    /// (reader count does not count upgradeable locks)
    fn on_read_unlock(&self, new_reader_count: usize);
    /// Called when an attempt to lock for reading has failed
    fn read_relax(&self);
    
    /// Called after the rwlock has unlocked from writing
    fn on_write_unlock(&self);
    /// Called when an attempt to lock for writing has failed
    fn write_relax(&self);
    
    /// Called after an upgradeable lock has been released
    fn on_upgradeable_release(&self);
    /// Called when an attempt to acquire an upgradeable lock has failed
    fn upgradeable_relax(&self);
    /// Called when an attempt to upgrade from upgradeable -> write has failed
    fn upgrade_u2w_relax(&self);
    
    /// Called after a write lock has been downgraded to a read lock
    fn on_downgrade_w2r(&self);
    /// Called after a write lock has been downgraded to an upgradeable lock
    fn on_downgrade_w2u(&self);
    /// Called after an upgradeable lock has been downgraded to a read lock
    fn on_downgrade_u2r(&self);
}
const WRITER: usize = 1<<56;
const UPGRADER: usize = 1<<48;
const EXCLUSIVE_THRESHOLD: usize = 1<<40;
pub struct BaseRwLockRaw<S:RwLockStrategy>(AtomicUsize,S);
impl<S:RwLockStrategy> BaseRwLockRaw<S> {
    /* If locked exclusively, returns Err(num_readers). Otherwise returns Ok(num_readers).
    Upgradeable locks (despite being readers) do not count towards this total. */
    pub fn reader_count(&self) -> Result<usize,usize> {
        let result = self.0.load(Ordering::Relaxed);
        if result >= EXCLUSIVE_THRESHOLD { return Err(result%EXCLUSIVE_THRESHOLD); }
        Ok(result)
    }
    /* If locked exclusively, returns true. Otherwise, returns false. */
    pub fn is_locked_exclusively(&self) -> bool {
        self.0.load(Ordering::Relaxed) >= EXCLUSIVE_THRESHOLD
    }
}
unsafe impl<S:RwLockStrategy> RawRwLock for BaseRwLockRaw<S> {
    type GuardMarker = GuardSend;
    const INIT: Self = Self(AtomicUsize::new(0),S::INIT);

    fn lock_shared(&self) {
        while !self.try_lock_shared() {
            // Failed
            while self.0.load(Ordering::Relaxed)>=EXCLUSIVE_THRESHOLD { self.1.read_relax(); }
        }
    }

    fn try_lock_shared(&self) -> bool {
        let value = self.0.fetch_add(1, Ordering::Acquire);
        if value>=EXCLUSIVE_THRESHOLD {
            let x = self.0.fetch_sub(1, Ordering::Release);
            self.1.on_read_unlock((x%EXCLUSIVE_THRESHOLD)-1);
            return false;
        }
        return true;
    }

    unsafe fn unlock_shared(&self) {
        let x = self.0.fetch_sub(1, Ordering::Release);
        self.1.on_read_unlock((x%EXCLUSIVE_THRESHOLD)-1)
    }

    fn lock_exclusive(&self) {
        while self.0.compare_exchange_weak(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.0.load(Ordering::Relaxed)!=0 { self.1.write_relax(); }
        }
    }

    fn try_lock_exclusive(&self) -> bool {
        self.0.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    unsafe fn unlock_exclusive(&self) {
        self.0.fetch_sub(WRITER, Ordering::Release);
        self.1.on_write_unlock();
    }
}
unsafe impl<S:RwLockStrategy> lock_api::RawRwLockDowngrade for BaseRwLockRaw<S> {
    unsafe fn downgrade(&self) {
        // Subtracting (WRITER-1) means that when x=WRITER, x will now equal 1 (a single shared lock), or so on
        self.0.fetch_sub(WRITER-1, Ordering::Release);
        self.1.on_downgrade_w2r();
    }
}
unsafe impl<S:RwLockStrategy> lock_api::RawRwLockUpgrade for BaseRwLockRaw<S> {
    fn lock_upgradable(&self) {
        while !self.try_lock_upgradable() {
            // Failed
            while self.0.load(Ordering::Relaxed)>=EXCLUSIVE_THRESHOLD { self.1.upgradeable_relax(); }
        }
    }
    fn try_lock_upgradable(&self) -> bool {
        let value = self.0.fetch_add(UPGRADER, Ordering::Acquire);
        if value>=EXCLUSIVE_THRESHOLD {  // (existing writer or upgrader)
            let x = self.0.fetch_sub(UPGRADER, Ordering::Release);
            return false;
        }
        return true;
    }
    
    unsafe fn unlock_upgradable(&self) {
        let x = self.0.fetch_sub(UPGRADER,Ordering::Release);
        self.1.on_upgradeable_release();
    }
    unsafe fn upgrade(&self) {
        while !self.try_upgrade() {
            // Failed
            while self.0.load(Ordering::Relaxed)!=UPGRADER { self.1.upgrade_u2w_relax(); }
        }
    }
    unsafe fn try_upgrade(&self) -> bool {
        self.0.compare_exchange(UPGRADER, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
}
unsafe impl<S:RwLockStrategy> lock_api::RawRwLockUpgradeDowngrade for BaseRwLockRaw<S> {
    unsafe fn downgrade_upgradable(&self) {
        self.0.fetch_sub(UPGRADER-1,Ordering::Release);
        self.1.on_downgrade_u2r();
    }
    unsafe fn downgrade_to_upgradable(&self) {
        self.0.fetch_sub(WRITER-UPGRADER,Ordering::Release);
        self.1.on_downgrade_w2u();
    }
}


pub type BaseRwLock<T,S> = lock_api::RwLock<BaseRwLockRaw<S>,T>;
pub type BaseRwLockReadGuard<'a,T,S> = lock_api::RwLockReadGuard<'a,BaseRwLockRaw<S>,T>;
pub type BaseRwLockWriteGuard<'a,T,S> = lock_api::RwLockWriteGuard<'a,BaseRwLockRaw<S>,T>;
pub type BaseRwLockUpgradableGuard<'a,T,S> = lock_api::RwLockUpgradableReadGuard<'a,BaseRwLockRaw<S>,T>;
pub type MappedBaseRwLockReadGuard<'a,T,S> = lock_api::MappedRwLockReadGuard<'a,BaseRwLockRaw<S>,T>;
pub type MappedBaseRwLockWriteGuard<'a,T,S> = lock_api::MappedRwLockWriteGuard<'a,BaseRwLockRaw<S>,T>;