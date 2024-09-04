//use super::{KRwLock,KRwLockReadGuard,MappedKRwLockReadGuard};
use lock_api::{RawRwLockDowngrade,RwLock,RwLockReadGuard,MappedRwLockReadGuard};
use alloc::vec::Vec;
use core::default::Default;
use crate::multitasking::get_cpu_num;
use core::ops::{Deref,DerefMut};
use crate::multitasking::without_interruptions;

pub struct CpuLocal<T: Default,L:RawRwLockDowngrade>(RwLock<L,Vec<T>>);
impl<T: Default,L:RawRwLockDowngrade> CpuLocal<T,L> {
    pub const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }
    
    fn _initialise_empty_values(&self, v: RwLockReadGuard<L,Vec<T>>, id: usize) -> RwLockReadGuard<L,Vec<T>> {
        drop(v);  // Drop the old guard so it doesn't block us
        let mut vu = self.0.write();  // currently cannot grab an upgradeable read as that can cause deadlocks in some rare cases
        while vu.len() <= id { vu.push(T::default()) };  // push new values so that `id` is a valid index
        lock_api::RwLockWriteGuard::downgrade(vu)  // downgrade back to a read guard now our job is done
    }
    
    /* Get the T for the CPU with the given number. */
    #[inline(always)]
    pub fn get_for(&self, id: usize) -> CpuLocalGuard<T,L> {
        let v = self.0.read();
        let v = if v.len() <= id {
            self._initialise_empty_values(v,id)
        } else { v };
        RwLockReadGuard::map(v, |v|&v[id])
    }
    
    /* Get the T for the current CPU. */
    #[inline(always)]
    pub fn get(&self) -> CpuLocalGuard<T,L> {
        self.get_for(get_cpu_num())
    }
}

// 'cl is CPULocal's lifetime
pub type CpuLocalGuard<'c1,T,L> = MappedRwLockReadGuard<'c1,L,T>;


// Utilities, since CpuLocal does not grant interior mutability due to obvious threading issues + borrow checker not liking nested guards

pub type CpuLocalLockedItem<T,L,LR> = CpuLocal<lock_api::Mutex<L,T>,LR>;
impl<L:lock_api::RawMutex,LR:RawRwLockDowngrade,T: Default> CpuLocalLockedItem<T,L,LR> {
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
pub type CpuLocalRWLockedItem<T,L> = CpuLocal<RwLock<L,T>,L>;
impl<L:RawRwLockDowngrade,T: Default> CpuLocalRWLockedItem<T,L> {
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
pub struct CpuLocalNoInterruptsLockedItem<T: Default>(pub CpuLocalLockedItem<T,super::KMutexRaw,super::KRwLockRaw>);
impl<T: Default> CpuLocalNoInterruptsLockedItem<T> {
    pub const fn new() -> Self { Self(CpuLocal::new()) }
    pub fn inspect<R>(&self, inspector: impl FnOnce(&T)->R) -> R {
        without_interruptions(||self.0.inspect(inspector))
    }
    pub fn mutate<R>(&self, mutator: impl FnOnce(&mut T)->R) -> R {
        without_interruptions(||self.0.mutate(mutator))
    }
}

pub type CpuLocalLockedOption<T,L,LR> = CpuLocalLockedItem<Option<T>,L,LR>;
impl<L:lock_api::RawMutex,LR:RawRwLockDowngrade,T> CpuLocalLockedOption<T,L,LR> {
    /// Equivalent of Option.insert(...), but does not return a mutable reference as doing that would violate lifetime rules
    #[inline]
    pub fn insert(&self, item: T){
        self.mutate(move |opt|{let _ = opt.insert(item);});
    }
    /// Equivalent of Option.insert(...), using a callback to recieve the mutable reference
    #[inline]
    pub fn insert_and<R>(&self, item: T, mutator: impl FnOnce(&mut T)->R) -> R {
        self.mutate(move |opt|{let r = opt.insert(item); mutator(r)})
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
    
    #[inline]
    pub fn inspect_unwrap<R>(&self, inspector: impl FnOnce(&T)->R) -> R {
        self.inspect(|opt|inspector(opt.as_ref().unwrap()))
    }
    #[inline]
    pub fn inspect_expect<R>(&self, inspector: impl FnOnce(&T)->R, errmsg: &str) -> R {
        self.inspect(|opt|inspector(opt.as_ref().expect(errmsg)))
    }
}
