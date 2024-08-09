use super::{RwLock,RwLockReadGuard};
use alloc::vec::Vec;
use core::default::Default;
use crate::multitasking::get_cpu_num;
use core::ops::{Deref,DerefMut};
use crate::lowlevel::without_interrupts;

pub struct CpuLocal<T: Default>(RwLock<Vec<T>>);
impl<T: Default> CpuLocal<T> {
    pub const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }
    
    fn _initialise_empty_values(&self, v: RwLockReadGuard<Vec<T>>, id: usize) -> RwLockReadGuard<Vec<T>> {
        let vu = self.0.upgradeable_read();  // Get an upgradeable read, blocking further readers and preventing writer starvation
        drop(v);  // Drop the old guard so it doesn't block us
        let mut vu = vu.upgrade();  // upgrade to a write guard
        while vu.len() <= id { vu.push(T::default()) };  // push new values so that `id` is a valid index
        vu.downgrade()  // downgrade back to a read guard now our job is done
    }
    
    /* Get the T for the CPU with the given number. */
    #[inline(always)]
    pub fn get_for(&self, id: usize) -> CpuLocalGuard<T> {
        let v = self.0.read();
        let v = if v.len() <= id {
            self._initialise_empty_values(v,id)
        } else { v };
        CpuLocalGuard(v, id)
    }
    
    /* Get the T for the current CPU. */
    #[inline(always)]
    pub fn get(&self) -> CpuLocalGuard<T> {
        self.get_for(get_cpu_num().into())
    }
}

// 'cl is CPULocal's lifetime
pub struct CpuLocalGuard<'cl,T>(RwLockReadGuard<'cl,Vec<T>>,usize);
impl<T> Deref for CpuLocalGuard<'_,T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &T {
        &self.0[self.1]
    }
}


// Utilities, since CpuLocal does not grant interior mutability due to obvious threading issues + borrow checker not liking nested guards
use super::Mutex;

pub type CpuLocalLockedItem<T> = CpuLocal<Mutex<T>>;
impl<T: Default> CpuLocalLockedItem<T> {
    /// Inspect (but do not mutate) the locked item
    #[inline]
    pub fn inspect<R>(&self, inspector: impl FnOnce(&T)->R) -> R {
        let cpul = self.get(); let item = cpul.lock();
        inspector(&item)
    }
    /// Inspect and mutate the locked item
    #[inline]
    pub fn mutate<R>(&self, mutator: impl FnOnce(&mut T)->R) -> R {
        let cpul = self.get(); let mut item = cpul.lock();
        mutator(&mut item)
    }
}
pub type CpuLocalRWLockedItem<T> = CpuLocal<RwLock<T>>;
impl<T: Default> CpuLocalRWLockedItem<T> {
    /// Inspect (but do not mutate) the locked item using a read guard
    #[inline]
    pub fn inspect<R>(&self, inspector: impl FnOnce(&T)->R) -> R {
        let cpul = self.get(); let item = cpul.read();
        inspector(&item)
    }
    /// Inspect and mutate the locked item using a write guard
    #[inline]
    pub fn mutate<R>(&self, mutator: impl FnOnce(&mut T)->R) -> R {
        let cpul = self.get(); let mut item = cpul.write();
        mutator(&mut item)
    }
}
pub struct CpuLocalNoInterruptsLockedItem<T: Default>(CpuLocalLockedItem<T>);
impl<T: Default> CpuLocalNoInterruptsLockedItem<T> {
    pub const fn new() -> Self { Self(CpuLocal::new()) }
    pub fn inspect<R>(&self, inspector: impl FnOnce(&T)->R) -> R {
        without_interrupts(||self.0.inspect(inspector))
    }
    pub fn mutate<R>(&self, mutator: impl FnOnce(&mut T)->R) -> R {
        without_interrupts(||self.0.mutate(mutator))
    }
}

pub type CpuLocalLockedOption<T> = CpuLocalLockedItem<Option<T>>;
impl<T> CpuLocalLockedOption<T> {
    /// Equivalent of Option.insert(...), but does not return a mutable reference as doing that would violate lifetime rules
    #[inline]
    pub fn insert(&self, item: T){
        self.mutate(move |opt|{let _ = opt.insert(item);});
    }
    /// Equivalent of Option.take()
    #[inline]
    pub fn take(&self) -> Option<T> {
        self.mutate(|opt|opt.take())
    }
    /// Equivalent of Option.replace(...)
    #[inline]
    pub fn replace(&self, item: T) -> Option<T> {
        self.mutate(move |opt|opt.replace(item))
    }
}
