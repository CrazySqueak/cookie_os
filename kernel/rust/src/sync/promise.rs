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
#[must_use = "If you wish to explicitly cancel the promise, consider using .cancel() instead." ]
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

// == ACKNOWLEDGEMENTS ==
struct AcknowledgementInner {
    state: AtomicU8,
    waiters: WaitingList,
}
const ACK_EMPTY: u8 = 0;
const ACK_POSITIVE: u8 = 1;
const ACK_NEGATIVE: u8 = 2;

/// An Acknowledgement is a special kind of promise, akin to a Promise<()>
/// However this version is more optimized (taking advantage of some improvements granted by having no data to write)
/// Note that Acknowledgements should not be used for synchronized access to values, as they use Ordering::Relaxed.
/// They are intended mainly for acknowledging that a value has been accepted by another task, for example
pub struct Acknowledgement(Arc<AcknowledgementInner>);
impl Acknowledgement {
    pub fn new() -> (AcknowledgementFulfiller,Acknowledgement) {
        let inner = AcknowledgementInner {
            state: AtomicU8::new(ACK_EMPTY),
            waiters: WaitingList::new(),
        };
        let ack = Acknowledgement(Arc::new(inner));
        let acf = AcknowledgementFulfiller(ack.clone());
        (acf, ack)
    }
    /// Block until the acknowledgement is filled.
    /// Returns true for a positive (complete())
    /// Returns false for a negative (cancel())
    pub fn get(&self) -> bool {
        self.0.waiters.wait_until_try(||{
            let x = self.0.state.load(Ordering::Relaxed);
            if x == ACK_EMPTY { None }
            else { Some(x == ACK_POSITIVE) }
        })
    }
    /// Return Ok(x) if the acknowledgement is filled (true for positive, false for negative)
    /// Return Err() if the acknowledgement is not yet filled
    pub fn try_get(&self) -> Result<bool,()> {
        match self.0.state.load(Ordering::Relaxed) {
            ACK_EMPTY => Err(()),
            ACK_POSITIVE => Ok(true),
            ACK_NEGATIVE => Ok(false),
            _ => unreachable!(),
        }
    }
    
    pub fn is_pending(&self) -> bool {
        self.0.state.load(Ordering::Relaxed) == ACK_EMPTY
    }
}
impl core::clone::Clone for Acknowledgement {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[must_use = "If you wish to explicitly cancel the acknowledgement, consider using .cancel() instead." ]
pub struct AcknowledgementFulfiller(Acknowledgement);
impl AcknowledgementFulfiller {
    pub fn acknowledgement(&self) -> &Acknowledgement {
        &self.0
    }
    
    fn _fill_internal(&self, value: u8) -> bool {
        let ok = self.0.0.state.compare_exchange(ACK_EMPTY, value, Ordering::Relaxed, Ordering::Relaxed).is_ok();
        if ok { self.0.0.waiters.notify_all() };
        ok
    }
    /// Returns true if successful, false if already filled
    pub fn complete(&self) -> bool {
        self._fill_internal(ACK_POSITIVE)
    }
    /// Returns true if successful, false if already filled
    pub fn cancel(&self) -> bool {
        self._fill_internal(ACK_NEGATIVE)
    }
}
impl core::ops::Drop for AcknowledgementFulfiller {
    fn drop(&mut self){
        let _ = self.cancel();
    }
}