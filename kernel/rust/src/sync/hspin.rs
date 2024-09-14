
use super::kspin;
use super::yspin;
use crate::multitasking::is_executing_task;
// (this whole implementation is a dirty hack implemented over KLocks)

macro_rules! inherit_lock_fn {
    ($vis:vis fn $fname:ident(&self) -> try($rt:ident<'_,T>)) => {
        $vis fn $fname(&self) -> Option<$rt<'_,T>> {
            self.0.$fname()
        }
    };
    ($vis:vis fn $fname:ident(&self) -> block($rt:ident<'_,T>; using $tfname:ident; relax while self.$relcond:ident())) => {
        $vis fn $fname(&self) -> $rt<'_,T> {
            loop {
                if let Some(guard) = self.$tfname() {
                    return guard;
                }
                // Relax
                relax(||self.$relcond());
            }
        }
    };
}

fn relax(relcond:impl Fn()->bool){
    use spin::RelaxStrategy;
    let can_yield = is_executing_task();
    while relcond() {
        if can_yield {
            // Yield to scheduler
            yspin::SchedulerYield::relax();
        } else {
            // Busy-loop
            spin::relax::Spin::relax();
        }
    }
}

pub struct HMutex<T>(kspin::KMutex<T>);
impl<T> HMutex<T> {
    pub const fn new(val: T) -> Self {
        Self(kspin::KMutex::new(val))
    }
    pub fn raw(&self) -> &super::baselocks::BaseMutexRaw<kspin::BlockingSpin> {
        self.0.raw()
    }
    
    inherit_lock_fn!(pub fn try_lock(&self) -> try(HMutexGuard<'_,T>));
    inherit_lock_fn!(pub fn lock(&self) -> block(HMutexGuard<'_,T>; using try_lock; relax while self.is_locked()));
    pub fn is_locked(&self) -> bool {
        self.raw().is_locked()
    }
}
pub type HMutexGuard<'a,T> = kspin::KMutexGuard<'a,T>;
pub type MappedHMutexGuard<'a,T> = kspin::MappedKMutexGuard<'a,T>;

pub struct HRwLock<T>(kspin::KRwLock<T>);
impl<T> HRwLock<T> {
    pub const fn new(val: T) -> Self {
        Self(kspin::KRwLock::new(val))
    }
    pub fn raw(&self) -> &super::baselocks::BaseRwLockRaw<kspin::BlockingSpin> {
        self.0.raw()
    }
    pub fn reader_count(&self) -> Result<usize,usize> {
        self.raw().reader_count()
    }
    pub fn is_locked_exclusively(&self) -> bool {
        self.raw().is_locked_exclusively()
    }
    fn is_locked_at_all(&self) -> bool {
        let readers = self.reader_count();
        readers.is_err() || readers.unwrap() > 0
    }
    
    pub unsafe fn force_unlock_read(&self) {
        self.0.force_unlock_read()
    }
    pub fn data_ptr(&self) -> *mut T {
        self.0.data_ptr()
    }
    
    inherit_lock_fn!(pub fn try_read(&self) -> try(HRwLockReadGuard<'_,T>));
    inherit_lock_fn!(pub fn read(&self) -> block(HRwLockReadGuard<'_,T>; using try_read; relax while self.is_locked_exclusively()));
    pub fn try_write(&self) -> Option<HRwLockWriteGuard<'_,T>> {
        self.0.try_write().map(|wg|HRwLockWriteGuard(wg))
    }
    inherit_lock_fn!(pub fn write(&self) -> block(HRwLockWriteGuard<'_,T>; using try_write; relax while self.is_locked_at_all()));
    
    pub fn try_upgradable_read(&self) -> Option<HRwLockUpgradableGuard<'_,T>> {
        self.0.try_upgradable_read().map(|ug|HRwLockUpgradableGuard(ug))
    }
    inherit_lock_fn!(pub fn upgradable_read(&self) -> block(HRwLockUpgradableGuard<'_,T>; using try_upgradable_read; relax while self.is_locked_exclusively()));
}
pub type HRwLockReadGuard<'a,T> = kspin::KRwLockReadGuard<'a,T>;
pub type MappedHRwLockReadGuard<'a,T> = kspin::MappedKRwLockReadGuard<'a,T>;
pub type MappedHRwLockWriteGuard<'a,T> = kspin::MappedKRwLockWriteGuard<'a,T>;

/// We have to use a wrapper type to be able to replace upgrade() with our own implementation
pub struct HRwLockUpgradableGuard<'a,T>(kspin::KRwLockUpgradableGuard<'a,T>);
impl<'a,T> HRwLockUpgradableGuard<'a,T> {
    pub fn rwlock_raw(s: &Self) -> &'a super::baselocks::BaseRwLockRaw<kspin::BlockingSpin> { kspin::KRwLockUpgradableGuard::rwlock_raw(&s.0) }
    
    pub fn try_upgrade(s: Self) -> Result<HRwLockWriteGuard<'a,T>,Self> {
        kspin::KRwLockUpgradableGuard::try_upgrade(s.0).map(|wg|HRwLockWriteGuard(wg)).map_err(|ug|Self(ug))
    }
    pub fn upgrade(mut s: Self) -> HRwLockWriteGuard<'a,T> {
        loop {
            match Self::try_upgrade(s) {
                Ok(wg) => return wg,
                Err(ug) => s = ug,
            }
            // Relax
            relax(||Self::rwlock_raw(&s).reader_count().unwrap_err()>0)
        }
    }
    
    pub fn downgrade(s: Self) -> HRwLockReadGuard<'a,T> {
        kspin::KRwLockUpgradableGuard::downgrade(s.0)
    }
}
impl<T> core::ops::Deref for HRwLockUpgradableGuard<'_,T> {
    type Target = T;
    fn deref(&self) -> &T {
        &*self.0
    }
}

/// And same for downgrade_upgradeable() for write guards
pub struct HRwLockWriteGuard<'a,T>(kspin::KRwLockWriteGuard<'a,T>);
impl<'a,T> HRwLockWriteGuard<'a,T> {
    pub fn rwlock_raw(s: &Self) -> &'a super::baselocks::BaseRwLockRaw<kspin::BlockingSpin> { kspin::KRwLockWriteGuard::rwlock_raw(&s.0) }
    
    pub fn downgrade(s: Self) -> HRwLockReadGuard<'a,T> {
        kspin::KRwLockWriteGuard::downgrade(s.0)
    }
    pub fn downgrade_to_upgradable(s: Self) -> HRwLockUpgradableGuard<'a,T> {
        HRwLockUpgradableGuard(kspin::KRwLockWriteGuard::downgrade_to_upgradable(s.0))
    }
    
    pub fn map<U>(s:Self,f:impl FnOnce(&mut T)->&mut U) -> MappedHRwLockWriteGuard<'a,U> {
        kspin::KRwLockWriteGuard::map(s.0,f)
    }
    pub fn try_map<U>(s:Self,f:impl FnOnce(&mut T)->Option<&mut U>) -> Result<MappedHRwLockWriteGuard<'a,U>,Self> {
        kspin::KRwLockWriteGuard::try_map(s.0,f).map_err(|wg|Self(wg))
    }
}
impl<T> core::ops::Deref for HRwLockWriteGuard<'_,T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
impl<T> core::ops::DerefMut for HRwLockWriteGuard<'_,T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}