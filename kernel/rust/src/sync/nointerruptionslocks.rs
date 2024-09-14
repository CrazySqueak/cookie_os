use crate::multitasking::interruptions::{NoInterruptionsGuard,disable_interruptions};
use core::ops::{Deref,DerefMut};
use super::baselocks::*;
use core::default::Default;

#[must_use]
pub struct NoInterruptionsGuardWrapper<G> {
    lock_guard: G,
    interrupt_guard: NoInterruptionsGuard, // <-- this lives at the end so that drop order ends the "no interruptions" stage as late as possible
}
impl<G> NoInterruptionsGuardWrapper<G> {
    pub fn new(lock_guard: G, interrupt_guard: NoInterruptionsGuard) -> Self {
        Self { lock_guard, interrupt_guard }
    }
    
    /// Return only the lock guard, dropping the NoInterruptionsGuard
    /// Safety: Care must be taken to ensure no interruptions occur that could cause issues with whatever item was locked
    pub unsafe fn into_lock_guard(s: Self) -> G {
        s.lock_guard
    }
    /// Return only the NoInterruptionsGuard, dropping the lock guard
    pub fn into_nointerrupt_guard(s: Self) -> NoInterruptionsGuard {
        s.interrupt_guard
    }
    /// Split both guards apart from each other.
    /// Safety: Care must be taken to ensure no interruptions occur that could cause issues with whatever item was locked
    pub unsafe fn into_separate_guards(s: Self) -> (G, NoInterruptionsGuard) {
        (s.lock_guard, s.interrupt_guard)
    }
}
impl<T,G> Deref for NoInterruptionsGuardWrapper<G> where G:Deref<Target=T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.lock_guard.deref()
    }
}
impl<T,G> DerefMut for NoInterruptionsGuardWrapper<G> where G:DerefMut<Target=T>, Self: Deref<Target=T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.lock_guard.deref_mut()
    }
}

macro_rules! ni_wrap_lock {
    ($vis:vis fn $fname:ident(&self) -> wrap($baset:ident<'_,T,S>)) => {
        #[cfg_attr(feature="dbg_track_nointerrupt_source", track_caller)]
        $vis fn $fname(&self) -> NoInterruptionsGuardWrapper<$baset<'_,T,S>> {
            let ni = disable_interruptions(); // (we have to disable them here to prevent a rare race condition where one could happen between locking and disabling interruptions)
            // Acquire the lock
            let guard = self.0.$fname();
            // Return
            NoInterruptionsGuardWrapper::new(guard,ni)
        }
    };
    ($vis:vis fn $fname:ident(&self) -> wrap_Option($baset:ident<'_,T,S>)) => {
        #[cfg_attr(feature="dbg_track_nointerrupt_source", track_caller)]
        $vis fn $fname(&self) -> Option<NoInterruptionsGuardWrapper<$baset<'_,T,S>>> {
            let ni = disable_interruptions(); // this will be dropped if try_lock fails, or kept if try_lock succeeds
            self.0.$fname().map(|guard|NoInterruptionsGuardWrapper::new(guard,ni))
        }
    };
}
macro_rules! ni_wrap_upgrade {
    ($vis:vis fn $fname:ident(s: Self) -> wrap($it:ident -> $rt:ident, <'a,T,S>)) => {
        $vis fn $fname(s: NoInterruptionsGuardWrapper<$it<'a,T,S>>) -> NoInterruptionsGuardWrapper<$rt<'a,T,S>> {
            let NoInterruptionsGuardWrapper{interrupt_guard: ni, lock_guard} = s;
            let lock_guard = $it::$fname(lock_guard);
            NoInterruptionsGuardWrapper::new(lock_guard, ni)
        }
    };
    ($vis:vis fn $fname:ident(s: Self) -> wrap_Result($it:ident -> $rt:ident, <'a,T,S>)) => {
        $vis fn $fname(s: NoInterruptionsGuardWrapper<$it<'a,T,S>>) -> Result<NoInterruptionsGuardWrapper<$rt<'a,T,S>>,Self> {
            let NoInterruptionsGuardWrapper{interrupt_guard: ni, lock_guard} = s;
            match $it::$fname(lock_guard) {
                Ok(lock_guard) => Ok(NoInterruptionsGuardWrapper::new(lock_guard, ni)),
                Err(lock_guard) => Err(NoInterruptionsGuardWrapper::new(lock_guard, ni)),
            }
        }
    };
}
macro_rules! ni_wrap_map {
    ($vis:vis fn $fname:ident(s:Self,f:F) -> wrap($st:ident->$mst:ident<'a,$rT:ty=>$rU:ty=(U),S>)) => {
        $vis fn $fname<U:?Sized>(s:Self,f:impl FnOnce($rT)->$rU) -> NoInterruptionsGuardWrapper<$mst<'a,U,S>> {
            let NoInterruptionsGuardWrapper{interrupt_guard: ni, lock_guard} = s;
            let lock_guard = $st::$fname(lock_guard,f);
            NoInterruptionsGuardWrapper::new(lock_guard, ni)
        }
    };
    ($vis:vis fn $fname:ident(s:Self,f:F) -> wrap_Result($st:ident->$mst:ident<'a,$rT:ty=>$rU:ty=(U),S>)) => {
        $vis fn $fname<U:?Sized>(s:Self,f:impl FnOnce($rT)->Option<$rU>) -> Result<NoInterruptionsGuardWrapper<$mst<'a,U,S>>,Self> {
            let NoInterruptionsGuardWrapper{interrupt_guard: ni, lock_guard} = s;
            match $st::$fname(lock_guard,f) {
                Ok(lock_guard) => Ok(NoInterruptionsGuardWrapper::new(lock_guard, ni)),
                Err(lock_guard) => Err(NoInterruptionsGuardWrapper::new(lock_guard, ni)),
            }
        }
    };
}

pub struct NIMutex<T,S:MutexStrategy>(BaseMutex<T,S>);
impl<T,S:MutexStrategy> NIMutex<T,S> {
    pub const fn new(val:T) -> Self {
        Self(BaseMutex::new(val))
    }
    pub fn raw(&self) -> &super::baselocks::BaseMutexRaw<S> {
        // SAFETY: .raw() being unsafe is redundant since raw().unlock() is already unsafe
        unsafe { self.0.raw() }
    }
    pub fn is_locked(&self) -> bool {
        self.raw().is_locked()
    }
    
    pub unsafe fn force_unlock(&self) {
        self.0.force_unlock()
    }
    pub fn data_ptr(&self) -> *mut T {
        self.0.data_ptr()
    }
    
    ni_wrap_lock!(pub fn lock(&self) -> wrap(BaseMutexGuard<'_,T,S>));
    ni_wrap_lock!(pub fn try_lock(&self) -> wrap_Option(BaseMutexGuard<'_,T,S>));
}
impl<T,S:MutexStrategy> Default for NIMutex<T,S> where BaseMutex<T,S>: Default {
    fn default() -> Self {
        Self(BaseMutex::default())
    }
}

impl<'a,T,S:MutexStrategy> NoInterruptionsGuardWrapper<BaseMutexGuard<'a,T,S>> {
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(BaseMutexGuard->MappedBaseMutexGuard<'a,&mut T=>&mut U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(BaseMutexGuard->MappedBaseMutexGuard<'a,&mut T=>&mut U=(U),S>));
}
impl<'a,T,S:MutexStrategy> NoInterruptionsGuardWrapper<MappedBaseMutexGuard<'a,T,S>> {
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(MappedBaseMutexGuard->MappedBaseMutexGuard<'a,&mut T=>&mut U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(MappedBaseMutexGuard->MappedBaseMutexGuard<'a,&mut T=>&mut U=(U),S>));
}

pub struct NIRwLock<T,S:RwLockStrategy>(BaseRwLock<T,S>);
impl<T,S:RwLockStrategy> NIRwLock<T,S> {
    pub const fn new(val:T) -> Self {
        Self(BaseRwLock::new(val))
    }
    pub fn raw(&self) -> &super::baselocks::BaseRwLockRaw<S> {
        // SAFETY: .raw() being unsafe is redundant since raw().unlock() is already unsafe
        unsafe { self.0.raw() }
    }
    pub fn reader_count(&self) -> Result<usize,usize> {
        self.raw().reader_count()
    }
    pub fn is_locked_exclusively(&self) -> bool {
        self.raw().is_locked_exclusively()
    }
    
    pub unsafe fn force_unlock_read(&self) {
        self.0.force_unlock_read()
    }
    pub fn data_ptr(&self) -> *mut T {
        self.0.data_ptr()
    }
    
    ni_wrap_lock!(pub fn read(&self) -> wrap(BaseRwLockReadGuard<'_,T,S>));
    ni_wrap_lock!(pub fn try_read(&self) -> wrap_Option(BaseRwLockReadGuard<'_,T,S>));
    ni_wrap_lock!(pub fn write(&self) -> wrap(BaseRwLockWriteGuard<'_,T,S>));
    ni_wrap_lock!(pub fn try_write(&self) -> wrap_Option(BaseRwLockWriteGuard<'_,T,S>));
    ni_wrap_lock!(pub fn upgradable_read(&self) -> wrap(BaseRwLockUpgradableGuard<'_,T,S>));
    ni_wrap_lock!(pub fn try_upgradable_read(&self) -> wrap_Option(BaseRwLockUpgradableGuard<'_,T,S>));
}
impl<T,S:RwLockStrategy> Default for NIRwLock<T,S> where BaseRwLock<T,S>: Default {
    fn default() -> Self {
        Self(BaseRwLock::default())
    }
}

impl<'a,T,S:RwLockStrategy> NoInterruptionsGuardWrapper<BaseRwLockUpgradableGuard<'a,T,S>> {
    pub fn rwlock_raw(s: &Self) -> &'a super::baselocks::BaseRwLockRaw<S> { unsafe{BaseRwLockUpgradableGuard::rwlock(&s.lock_guard).raw()} }  // SAFETY: raw() being unsafe is redundant
    ni_wrap_upgrade!(pub fn upgrade(s: Self) -> wrap(BaseRwLockUpgradableGuard -> BaseRwLockWriteGuard, <'a,T,S>));
    ni_wrap_upgrade!(pub fn try_upgrade(s: Self) -> wrap_Result(BaseRwLockUpgradableGuard -> BaseRwLockWriteGuard, <'a,T,S>));
    ni_wrap_upgrade!(pub fn downgrade(s: Self) -> wrap(BaseRwLockUpgradableGuard -> BaseRwLockReadGuard, <'a,T,S>));
}
impl<'a,T,S:RwLockStrategy> NoInterruptionsGuardWrapper<BaseRwLockWriteGuard<'a,T,S>> {
    pub fn rwlock_raw(s: &Self) -> &'a super::baselocks::BaseRwLockRaw<S> { unsafe{BaseRwLockWriteGuard::rwlock(&s.lock_guard).raw()} }  // SAFETY: raw() being unsafe is redundant
    ni_wrap_upgrade!(pub fn downgrade(s: Self) -> wrap(BaseRwLockWriteGuard -> BaseRwLockReadGuard, <'a,T,S>));
    ni_wrap_upgrade!(pub fn downgrade_to_upgradable(s: Self) -> wrap(BaseRwLockWriteGuard -> BaseRwLockUpgradableGuard, <'a,T,S>));
    
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(BaseRwLockWriteGuard->MappedBaseRwLockWriteGuard<'a,&mut T=>&mut U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(BaseRwLockWriteGuard->MappedBaseRwLockWriteGuard<'a,&mut T=>&mut U=(U),S>));
}
impl<'a,T,S:RwLockStrategy> NoInterruptionsGuardWrapper<MappedBaseRwLockWriteGuard<'a,T,S>> {
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(MappedBaseRwLockWriteGuard->MappedBaseRwLockWriteGuard<'a,&mut T=>&mut U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(MappedBaseRwLockWriteGuard->MappedBaseRwLockWriteGuard<'a,&mut T=>&mut U=(U),S>));
}
impl<'a,T,S:RwLockStrategy> NoInterruptionsGuardWrapper<BaseRwLockReadGuard<'a,T,S>> {
    pub fn rwlock_raw(s: &Self) -> &'a super::baselocks::BaseRwLockRaw<S> { unsafe{BaseRwLockReadGuard::rwlock(&s.lock_guard).raw()} }  // SAFETY: raw() being unsafe is redundant
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(BaseRwLockReadGuard->MappedBaseRwLockReadGuard<'a,&T=>&U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(BaseRwLockReadGuard->MappedBaseRwLockReadGuard<'a,&T=>&U=(U),S>));
}
impl<'a,T,S:RwLockStrategy> NoInterruptionsGuardWrapper<MappedBaseRwLockReadGuard<'a,T,S>> {
    ni_wrap_map!(pub fn map(s:Self,f:F) -> wrap(MappedBaseRwLockReadGuard->MappedBaseRwLockReadGuard<'a,&T=>&U=(U),S>));
    ni_wrap_map!(pub fn try_map(s:Self,f:F) -> wrap_Result(MappedBaseRwLockReadGuard->MappedBaseRwLockReadGuard<'a,&T=>&U=(U),S>));
}

pub type BaseNoInterruptionsMutex<T,S> = NIMutex<T,S>;
pub type BaseNoInterruptionsMutexGuard<'a,T,S> = NoInterruptionsGuardWrapper<BaseMutexGuard<'a,T,S>>;
pub type MappedBaseNoInterruptionsMutexGuard<'a,T,S> = NoInterruptionsGuardWrapper<MappedBaseMutexGuard<'a,T,S>>;

pub type BaseNoInterruptionsRwLock<T,S> = NIRwLock<T,S>;
pub type BaseNoInterruptionsRwLockReadGuard<'a,T,S> = NoInterruptionsGuardWrapper<BaseRwLockReadGuard<'a,T,S>>;
pub type BaseNoInterruptionsRwLockWriteGuard<'a,T,S> = NoInterruptionsGuardWrapper<BaseRwLockWriteGuard<'a,T,S>>;
pub type BaseNoInterruptionsRwLockUpgradableGuard<'a,T,S> = NoInterruptionsGuardWrapper<BaseRwLockUpgradableGuard<'a,T,S>>;
pub type MappedBaseNoInterruptionsRwLockReadGuard<'a,T,S> = NoInterruptionsGuardWrapper<MappedBaseRwLockReadGuard<'a,T,S>>;
pub type MappedBaseNoInterruptionsRwLockWriteGuard<'a,T,S> = NoInterruptionsGuardWrapper<MappedBaseRwLockWriteGuard<'a,T,S>>;
