//! Promises are used to "promise" a result from one thread to another

use core::sync::atomic::*;
use super::WaitingList;
use core::cell::SyncUnsafeCell;
use core::mem::MaybeUninit;
use alloc::sync::Arc;

const PROMISE_EMPTY: u8 = 0;  // empty/unfulfilled
const PROMISE_INUSE: u8 = 1;  // being written to
const PROMISE_COMPLETE: u8 = 2;  // completed (either fulfilled or cancelled)

/// A OnceLock used for implementing promises.
/// Also suitable for other uses as well.
pub struct POnceLock<T> {
    state: AtomicU8,
    value: SyncUnsafeCell<Option<T>>,
}
impl<T> POnceLock<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(PROMISE_EMPTY),
            value: SyncUnsafeCell::new(None),
        }
    }
    
    /// Get the value, or None if not yet set
    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == PROMISE_COMPLETE {
            // The value has been filled
            // Safety: This cannot be written to once the state is set to COMPLETE
            // So we know that a value is prent
            let value = unsafe { &*self.value.get() };
            Some(value.as_ref().unwrap())  // (if the Option is still None then something has gone terribly wrong)
        } else { None }
    }
    /// Set the value, if it has not already been set
    pub fn set(&self, value: T) -> Result<(),T> {
        // Set state to IN-USE if EMPTY, otherwise return err
        match self.state.compare_exchange(PROMISE_EMPTY,PROMISE_INUSE,Ordering::Acquire,Ordering::Relaxed) {
            Err(_) => Err(value),
            
            // We now hold an "exclusive write lock", as future compare_exchanges will fail as the state is now INUSE instead of EMPTY
            Ok(_) => {
                unsafe { *self.value.get() = Some(value) };
                // Now, specify that we've filled the oncelock
                self.state.store(PROMISE_COMPLETE, Ordering::Release);
                // And return Ok()
                Ok(())
            }
        }
    }
    
    /// Returns true if the value has been filled (get() would return Some())
    pub fn is_filled(&self) -> bool {
        self.state.load(Ordering::Relaxed) == PROMISE_COMPLETE
    }
    /// Returns true if the value is empty (set() would return Ok(), provided nothing sets it in-between your calls to is_empty() and set())
    /// N.B. This is not equivalent to the inverse of is_filled, as is_filled()==false && is_empty()==false is possible if the cell is currently being filled (written to)
    pub fn is_empty(&self) -> bool {
        self.state.load(Ordering::Relaxed) == PROMISE_EMPTY
    }
}
impl<T> core::default::Default for POnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
}

struct PromiseInner<T> {
    value: POnceLock<Option<T>>,  // Some = Success, None = Failure
    waiters: WaitingList,
}

pub struct Promise<T>(Arc<PromiseInner<T>>);
impl<T> Promise<T> {
    /// Create a new promise, returning both a fulfiller to fulfill it, and a copy of the Promise
    pub fn new() -> (PromiseFulfiller<T>,Promise<T>) {
        let inner: PromiseInner<T> = PromiseInner {
            value: POnceLock::new(),
            waiters: WaitingList::new(),
        };
        let promise = Self(Arc::new(inner));
        let fulfiller = PromiseFulfiller(promise.clone());
        (fulfiller, promise)
    }
    
    /// Get the result of the promise, blocking until fulfilled.
    /// If the PromiseFulfiller was dropped without completing the promise, returns Err()
    pub fn get(&self) -> Result<&T,()> {
        let value = self.0.waiters.wait_until_try(||self.0.value.get());  // wait until the cell is filled
        value.as_ref().ok_or(())
    }
    /// Try to get the result of the promise. Returns Err(false) if the promise is not yet completed, and Err(true) if the promise has been cancelled.
    pub fn try_get(&self) -> Result<&T,bool> {
        match self.0.value.get() {
            Some(Some(value)) => Ok(value),
            Some(None) => Err(true),  // cancelled
            None => Err(false),  // not filled yet
        }
    }
    
    /// Returns true if the promise is not yet fulfilled or cancelled (i.e. it's pending, and a call to get() would block)
    /// Note: No synchronization guarantees are made by this method. To try and get the value, failing if pending, use try_get().
    pub fn is_pending(&self) -> bool {
        !self.0.value.is_filled()
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
    
    fn _fill_internal(&self, value: Option<T>) -> Result<(),Option<T>> {
        match self.0.0.value.set(value) {
            Ok(_) => {
                // Success
                // Remember to wake any waiting threads
                self.0.0.waiters.notify_all();
                // And return
                Ok(())
            }
            // Failed. already filled. all waiting threads already awake
            Err(x) => Err(x),
        }
    }
    
    /// Complete the promise with the given value.
    /// Returns Ok() if the promise was successfully filled, Err(original) if the promise had already been completed
    pub fn complete(&self, value: T) -> Result<(),T> {
        self._fill_internal(Some(value)).map_err(|v|v.unwrap())
    }
    
    /// Cancel the promise early. The promise is automatically cancelled if the fulfiller is dropped when the promise is unfilled, but using this method allows you to cancel it early.
    /// Returns Ok() if the promise was successfully cancelled, Err() if the promise had already been filled
    pub fn cancel(&self) -> Result<(),()> {
        self._fill_internal(None).map_err(|_|())
    }
}
impl<T> core::ops::Drop for PromiseFulfiller<T> {
    fn drop(&mut self){
        // Cancel the promise if necessary
        let _ = self.cancel();
    }
}
