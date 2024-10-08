//use super::{KRwLock,KRwLockReadGuard,MappedKRwLockReadGuard};

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::default::Default;
use core::ptr::NonNull;
use super::get_cpu_num;
type RwLock<T> = crate::sync::kspin::KRwLock<T>;

/// T - the type of the value
/// 'a - the lifetime of the value
/// SHARED - If true, other CPUs may use get_for to acquire a reference to the value for another CPU
/// (items must still be Sync as multiple threads may run on the same CPU)
pub struct CpuLocal<T: Default + ?Sized, const SHARED: bool>(RwLock<Vec<Option<NonNull<T>>>>);
impl<T: Default + ?Sized,const SHARED: bool> CpuLocal<T,SHARED> {
    pub const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }
    
    #[inline]
    fn _initialise_empty(&self, up_to_id_inclusive: usize) {
        let mut pointers = self.0.write();
        while pointers.len() <= up_to_id_inclusive {
            // Push a null pointer to our vec
            pointers.push(None);
        }
    }
    
    fn _get_for_inner(&self, id: usize) -> &T {
        let rg = self.0.read();
        let item = if rg.len() <= id {
            // Add null pointers if needed
            drop(rg);
            self._initialise_empty(id);
            self.0.read()[id]
        } else {
            let item = rg[id];
            drop(rg);
            item
        };

        let item: *const T = match item {
            Some(ptr) => ptr.as_ptr(),
            None => {
                let mut wg = self.0.write();
                // Create a new item in a box
                let item_box = Box::new(T::default());
                // Convert to raw pointer
                let item_ref = Box::into_raw(item_box);
                // Store in vec
                wg[id] = NonNull::new(item_ref);
                // Release write lock
                drop(wg);
                // And return
                item_ref

                // CpuLocals have generally been used in statics, where their values live for the remainder of the program
                // Therefore, by allocating on the heap and storing a shared reference, we only need to keep a read guard long enough to obtain a reference
                // Rather than having to keep it for the duration we spend referencing the value
                // (thus reducing contention)
            },
        };

        // SAFETY: The pointer is guaranteed to live as long as we do
        //          and in the signature, it's elided for the reference to not outlive us.
        //          It is impossible to obtain a mutable reference to an item in a CPU local.
        //          Therefore, it should be safe to turn our pointer into a reference.
        unsafe { &*item }
    }
    #[inline(always)]
    pub fn get_current(x: &Self) -> &T  {
        x._get_for_inner(get_cpu_num())
    }
}
impl<T: Default + ?Sized> CpuLocal<T,true> {
    #[inline(always)]
    pub fn get_for(x: &Self, id: usize) -> &T {
        x._get_for_inner(id)
    }

    pub fn get_all_cpus(x: &Self) -> Vec<(usize,&T)> {
        x.0.read().iter().enumerate()
            .flat_map(|(i,o)|o.map(|x|(i,x)))
            .map(|(i,ptr)|(i,unsafe{ptr.as_ref()})).collect()
    }
}
impl<T: Default + ?Sized,const SHARED: bool> core::ops::Deref for CpuLocal<T,SHARED> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        Self::get_current(self)
    }
}
impl<T: Default + ?Sized,const SHARED: bool> Drop for CpuLocal<T,SHARED> {
    fn drop(&mut self) {
        // Drop all contained values
        let inner_mut = self.0.get_mut();
        for ptr in inner_mut.iter_mut() {
            match ptr.take() {
                Some(ptr) => unsafe {
                    // Drop the pointed-to value
                    // SAFETY: We've used ptr.take() to ensure that the pointer is null-ed after
                    //          so it cannot be dropped twice by mistake.
                    //         As this is running in a Drop impl, and our getter methods ensure that
                    //          their references are bound by our lifetime, we can be sure that we are the only
                    //          reference to the contained value, so it may safely be dropped.
                    let contained = Box::from_raw(ptr.as_ptr());
                    drop(contained)  // drop the box, de-allocating the contained value
                },
                None => {},  // already empty
            }
        }
    }
}
// SAFETY: Much like Carton (in the rustonomicon https://doc.rust-lang.org/nomicon/send-and-sync.html)
//          CpuLocal maintains ownership over the contained value. Sending it between threads is valid if T can be sent between threads.
unsafe impl<T: Default + ?Sized,const SHARED: bool> Send for CpuLocal<T,SHARED> where T: Send {}
// SAFETY: CpuLocal does not allow mutable access to its contents (users must use their own interior mutability primitive such as a mutex)
//          It doesn't allow mutating its contents, but does allow accessing them as an immutable reference.
//          Therefore, it may be Sync if T is Sync
unsafe impl<T: Default + ?Sized,const SHARED: bool> Sync for CpuLocal<T,SHARED> where T: Sync {}