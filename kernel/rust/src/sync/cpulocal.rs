use super::{RwLock,RwLockReadGuard};
use alloc::vec::Vec;
use core::default::Default;
use crate::multitasking::get_cpu_num;
use core::ops::{Deref,DerefMut};

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
