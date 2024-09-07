//use super::{KRwLock,KRwLockReadGuard,MappedKRwLockReadGuard};

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::default::Default;
use super::get_cpu_num;
// TODO: KMutex once i implement proper sync primitives
//      (we don't need to use LLMutex as LLMutex indirectly depends on fixedcpulocal, not dynamic cpu locals
type RwLock<T> = spin::RwLock<T>;

/// T - the type of the value
/// 'a - the lifetime of the value
/// SHARED - If true, other CPUs may use get_for to acquire a reference to the value for another CPU
/// (items must still be Sync as multiple threads may run on the same CPU)
pub struct CpuLocal<'a, T: Default + ?Sized + 'a, const SHARED: bool>(RwLock<Vec<&'a T>>);
impl<'a,T: Default + ?Sized + 'a,const SHARED: bool> CpuLocal<'a,T,SHARED> {
    pub const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }
    
    #[inline]
    fn _initialise_empty(&self, up_to_id_inclusive: usize) {
        let mut references = self.0.write();
        while references.len() <= up_to_id_inclusive {
            // Create a new item in a box
            let item_box = Box::new(T::default());
            // Leak and reborrow as immutable
            let item_ref: &'a T = &*Box::leak(item_box);
            // Push to our vec
            references.push(item_ref);
            
            // CpuLocals have generally been used in statics, where their values live for the remainder of the program
            // Therefore, by allocating on the heap and storing a shared reference, we only need to keep a read guard long enough to obtain a reference
            // Rather than having to keep it for the duration we spend referencing the value
            // (thus reducing contention)
        }
    }
    
    fn _get_for_inner(&self, id: usize) -> &'a T {
        let rg = self.0.read();
        let item = if rg.len() <= id {
            drop(rg);
            self._initialise_empty(id);
            (self.0.read())[id]
        } else {
            rg[id]
        };
        item
    }
    #[inline(always)]
    pub fn get_current(x: &CpuLocal<'a,T,SHARED>) -> &'a T {
        x._get_for_inner(get_cpu_num())
    }
}
impl<'a,T: Default + ?Sized + 'a> CpuLocal<'a,T,true> {
    #[inline(always)]
    pub fn get_for(x: &CpuLocal<'a,T,true>, id: usize) -> &'a T {
        x._get_for_inner(id)
    }
}
impl<'a,T: Default + ?Sized,const SHARED: bool> core::ops::Deref for CpuLocal<'a,T,SHARED> where Self: 'a {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        Self::get_current(self)
    }
}

/*
use lock_api::{RawRwLockDowngrade,RwLock,RwLockReadGuard,MappedRwLockReadGuard};
use crate::sync::kspin::KRwLock;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::default::Default;
use crate::multitasking::get_cpu_num;
use core::ops::{Deref,DerefMut};
use crate::multitasking::without_interruptions;

pub struct CpuLocal<T: Default>(KRwLock<Vec<&'static T>>);
impl<T: Default,L:RawRwLockDowngrade> CpuLocal<T,L> {
    pub const fn new() -> Self {
        Self(KRwLock::new(Vec::new()))
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
pub type CpuLocalGuard<'c1,T,> = MappedRwLockReadGuard<'c1,T>;


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
pub struct CpuLocalNoInterruptsLockedItem<T: Default>(pub CpuLocalLockedItem<T,super::kspin::KMutexRaw,super::kspin::KRwLockRaw>);
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

*/