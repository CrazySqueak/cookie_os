//! Promises are used to "promise" a result from one thread to another

use core::sync::atomic::*;
use super::WaitingList;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use alloc::sync::Arc;

const PROMISE_EMPTY: u8 = 0;  // empty/unfulfilled
const PROMISE_INUSE: u8 = 1;  // being written to
const PROMISE_COMPLETE: u8 = 2;  // completed (either fulfilled or cancelled)

struct PromiseInner<T> {
    state: AtomicU8,
    value: SyncUnsafeCell<Option<T>>,
    waiters: WaitingList,
}

pub struct Promise<T>(Arc<PromiseInner<T>>);
impl<T> Promise<T> {
    /// Create a new promise, returning both a fulfiller to fulfill it, and a copy of the Promise
    pub fn new() -> (PromiseFulfiller<T>,Promise<T>) {
        let inner: PromiseInner<T> = PromiseInner {
            state: AtomicU8::new(PROMISE_EMPTY),
            value: SyncUnsafeCell::new(None),
            waiters: WaitingList::new(),
        };
        let promise = Self(Arc::new(inner));
        let fulfiller = PromiseFulfiller(promise.clone());
        (fulfiller, promise)
    }
    
    /// Get the result of the promise, blocking until fulfilled.
    /// If the PromiseFulfiller was dropped without completing the promise, returns Err()
    pub fn get(&self) -> Result<&T,()> {
        self.0.waiters.wait_until(||self.0.state.load(Ordering::Acquire)==PROMISE_COMPLETE);
        // SAFETY: We now know that the promise is either fulfilled or cancelled. (thus we have read access)
        let value = unsafe { &*self.0.value.get() };
        value.as_ref().ok_or(())  // Return Ok() if fulfilled or Err() if cancelled
    }
    /// Try to get the result of the promise. Returns Err(false) if the promise is not yet completed, and Err(true) if the promise has been cancelled.
    pub fn try_get(&self) -> Result<&T,bool> {
        if self.0.state.load(Ordering::Acquire)==PROMISE_COMPLETE {
            // Promise completed
            // safety: we did the atomic load so we know it's completed and not still in progress of being written
            let value = unsafe { &*self.0.value.get() };
            value.as_ref().ok_or(true)  // Ok() if fulfilled, Err(true) if cancelled
        } else { Err(false) }
    }
    
    /// Returns true if the promise is not yet fulfilled or cancelled (i.e. it's pending, and a call to get() would block)
    /// Note: No synchronization guarantees are made by this method. To try and get the value, failing if pending, use try_get().
    pub fn is_pending(&self) -> bool {
        self.0.state.load(Ordering::Relaxed)!=PROMISE_COMPLETE
    }
}
impl<T> core::clone::Clone for Promise<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// A write guard responsible for fulfilling a promise
/// Only one of these may exist as it has a Drop impl,
/// however complete() is thread-safe and works from an immutable reference.
pub struct PromiseFulfiller<T>(Promise<T>);
impl<T> PromiseFulfiller<T> {
    /// Return a reference to the promise this fulfills
    /// (call .clone() on it to clone the underlying arc and get your own reference)
    pub fn promise(&self) -> &Promise<T> {
        &self.0
    }
    
    /// Complete the promise with the given value.
    /// Returns Ok() if the promise was successfully filled, Err(original) if the promise had already been completed
    pub fn complete(&self, value: T) -> Result<(),T> {
        match self.0.0.state.compare_exchange(PROMISE_EMPTY,PROMISE_INUSE,Ordering::Acquire,Ordering::Relaxed) {
            Err(_) => Err(value),  // (already filled or in use)
            
            // We now hold an "exclusive write lock" of sorts
            Ok(_) => {
                // SAFETY: the compare_exchange ensures that we have exclusive access to the value
                unsafe{ *self.0.0.value.get() = Some(value) }
                // Now, we update the atomic to specify that we've completed the promise
                self.0.0.state.store(PROMISE_COMPLETE,Ordering::Release);
                // And wake anyone on the waiting list
                self.0.0.waiters.notify_all();
                Ok(())
            },
        }
    }
    
    /// Cancel the promise early. The promise is automatically cancelled if the fulfiller is dropped when the promise is unfilled, but using this method allows you to cancel it early.
    /// Returns Ok() if the promise was successfully cancelled, Err() if the promise had already been filled
    pub fn cancel(&self) -> Result<(),()> {
        match self.0.0.state.compare_exchange(PROMISE_EMPTY,PROMISE_COMPLETE,Ordering::Acquire,Ordering::Relaxed) {
            // (we can jump straight from empty -> complete as value defaults to None)
            Err(_) => Err(()),
            Ok(_) => {
                // Success: Notify all waiters
                self.0.0.waiters.notify_all();
                // And then get out of here
                Ok(())
            }
        }
    }
}
impl<T> core::ops::Drop for PromiseFulfiller<T> {
    fn drop(&mut self){
        // Update the promise from EMPTY to CANCEL
        // Note: We have a mut reference so we know it isn't being written to (INUSE) elsewhere
        let state = self.0.0.state.load(Ordering::Relaxed);
        if state != PROMISE_COMPLETE {
            // We've been dropped and the promise was incomplete.
            // Mark it as cancelled...
            self.0.0.state.store(PROMISE_COMPLETE, Ordering::Release);
            // ...and wake the waiters
            self.0.0.waiters.notify_all();
        }
    }
}

/// A OnceLock implemented as a wrapper around a Promise
/// Note: The fact that PromiseFulfiller contains a copy of the promise is currently an implementation detail, so we must store both a fulfiller and a reader
pub struct POnceLock<T> { w: PromiseFulfiller<T>, r: Promise<T> }
impl<T> POnceLock<T> {
    pub fn new() -> Self {
        let (w, r) = Promise::<T>::new();
        return Self { w, r };
    }
    
    /// Get the value, or None if not yet set
    pub fn get(&self) -> Option<&T> {
        self.r.try_get().ok()
    }
    /// Set the value, if it has not already been set
    pub fn set(&self, value: T) -> Result<(),T> {
        self.w.complete(value)
    }
    
    /// Get the value, blocking (using a WaitingList) until it is available.
    /// This is equivalent to just waiting on a promise.
    pub fn get_blocking(&self) -> &T {
        self.r.get().unwrap()
    }
}
impl<T> core::default::Default for POnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
}