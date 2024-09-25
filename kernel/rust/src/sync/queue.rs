
use super::{WaitingList,YMutex};
use alloc::collections::VecDeque;

/// A synchronized queue.
pub struct WQueue<T> {
    waiters: WaitingList,
    queue: YMutex<VecDeque<T>>,
}
impl<T> WQueue<T> {
    /// Get an item from the queue, blocking until one is available
    pub fn get(&self) -> T {
        loop{
            // Attempt to get an item from the queue
            let item = self.queue.lock().pop_front();
            if let Some(item) = item { return item; }  // Return if successful
            // Wait until the queue is not empty
            self.waiters.wait_until(||!self.queue.lock().is_empty());
        }
    }
    /// Try and get an item from the queue. Blocks until the queue is unlocked, returns Some() if one was there or None if the queue was empty.
    pub fn get_if_available(&self) -> Option<T> {
        self.queue.lock().pop_front()
    }
    /// Try and get an item from the queue. May spuriously return None if the queue is locked (instead of blocking until it's unlocked like get_if_available).
    pub fn get_nonblocking(&self) -> Option<T> {
        self.queue.try_lock().map(|mut q|q.pop_front()).flatten()
    }
    
    /// Push an item to the queue
    pub fn push(&self, item: T) {
        // Push the item to the queue
        let mut wg = self.queue.lock();
        wg.push_back(item);
        drop(wg);
        // And wake a waiting thread
        self.waiters.notify_one();
    }
}