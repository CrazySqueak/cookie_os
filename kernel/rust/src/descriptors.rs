use core::sync::atomic::{AtomicU16,AtomicU64,Ordering,AtomicPtr};
use alloc::sync::Arc;
use core::ops::Drop;
use core::cell::SyncUnsafeCell;
use core::mem::{forget};

/** Descriptors are objects with are opened and stored in a descriptor table.
    These are a lot of kernel managed objects, including things such as processes, open files, and more.
    
    There are two types of references that may be opened: A-references and B-references. A-references only count for rc_a, whereas B-references count for both rc_a and rc_b.
    The difference between the two depends on the descriptor.
    
    Each descriptor contains three descriptor-specific sections: the table section, the A section, and the B section.
    Once no more B-references exist, the B section is dropped. Once no more A-references exist, the A section is dropped.
    The table section is never "dropped", but is overwritten when a new descriptor is assigned to that slot in the table.
    
    The rc_a count may be one of the following values: 0 - free, 1 - reserved (no other references may be taken), 2+ - initialised
    The rc_b value is either 0 - deallocated, or 1+ - allocated. This is because there is no need for a "free" value in rc_b to signify that the descriptor is no longer in use, as that responsibility is handled by rc_a.
    TODO: determine how re-opening a b reference works (if possible)
*/
pub struct Descriptor<T,A,B> {
    // Ref counts
    rc_a: AtomicU16,
    rc_b: AtomicU16,
    
    // ID
    id: AtomicU64,
    
    // Sections
    // Null pointers mean the value there has been dropped
    slot_t: T,
    slot_a: SyncUnsafeCell<Option<A>>,
    slot_b: SyncUnsafeCell<Option<B>>,
}
impl<T,A,B> Descriptor<T,A,B> {
    /* Drop the slot_a value if non-null, then decrement rc_a to 0, marking this descriptor as free to be overwritten.
        SAFETY: rc_a should be 1. This MUST be the only thread trying to access this descriptor (enforced by rc_a being 1).
                rc_b must be 0 and slot_b must be freed. slot_t must not be in use (lifetimes should be pinned to the reference, so they do not outlive their A-references).
                When clearing the entire descriptor, you should clear slot B (_clear_slot_b) first, then call _clear. */
    unsafe fn _clear(&self){
        // Drop slot_a if initialised
        let slot_a = &mut *self.slot_a.get();
        slot_a.take();  // sets slot_a to None and drops the previous value
        
        // Now that slot_a is cleared, we decrement rc_a to 0. The descriptor is now free to be overwritten.
        self.rc_a.store(0, Ordering::Release);
    }
    /* Drop the slot_b value if non-null.
        SAFETY: rc_b should be 0. This MaybeUninit be the only thread trying to access this descriptor (enforced by rc_a being 1). */
    unsafe fn _clear_slot_b(&self){
        // Drop slot_b if initialised
        let slot_b = &mut *self.slot_b.get();
        slot_b.take();  // sets slot_b to None and drops the previous value
    }
    
    /* Put the given two values into slots A and B.
       SAFETY: rc_a should be 1 and rc_b should be 0.
                This MUST be the only thread trying to access this descriptor.*/
    unsafe fn _init_slots(&self, a_value: A, b_value: B){
        // Init slot A
        let slot_a = &mut *self.slot_a.get();
        let _=slot_a.insert(a_value);
        // Init slot B
        let slot_b = &mut *self.slot_b.get();
        let _=slot_b.insert(b_value);
        // Done :)
    }
    
    /* Reserve the descriptor for use.
       This will increment rc_a from 0 (free) to 1 (reserved).
       Returns None if the operation failed (e.g. because the descriptor is already in use). */
    pub fn reserve(&self, id: u64) -> Option<DescriptorInitialiser<T,A,B>> {
        // Attempt to begin initialisation by compare_exchange-ing the rc_a value.
        let r = self.rc_a.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed);
        if let Err(_) = r { return None; }  // If the compare_exchange failed, then the descriptor is already in use, so we return None.
        self.id.store(id, Ordering::Relaxed);  // save the descriptor ID?
        // rc_a is now equal to 1 (reserved). This therefore signifies that we are the only one currently using it, as all attempts to use it will now fail.
        Some(DescriptorInitialiser(self))
    }
}

/* An RAII guard used for initialising a descriptor.
    If commit() is called, the descriptor's rc_a will be incremented to 2, and its descriptor will become ready for use.
    If this is otherwise dropped, the descriptor will be cleared and have its rc_a decremented back to 0 (free). */
pub struct DescriptorInitialiser<'a,T,A,B>(&'a Descriptor<T,A,B>);
impl<T,A,B> DescriptorInitialiser<'_,T,A,B> {
    pub fn id(&self) -> u64 { self.0.id.load(Ordering::Relaxed) }
    pub fn slot_t<'a>(&'a self) -> &'a T { &self.0.slot_t }
    
    /* Finish the initialisation of the descriptor, putting a_value into slot a, b_value into slot b, and eventually incrementing its rc_a count to 2, its rc_b count to 1, and returning a B-reference.
        Once this method is run, the descriptor may have any number of references taken in the future, and we no longer exclusively own it. */
    pub fn commit(self, a_value: A, b_value: B) -> () {
        let descriptor = self.0;
        // SAFETY: As we are the only reference to the descriptor (it is still being initialised), we can ensure that
        // the required safety invariants are met.
        unsafe { descriptor._init_slots(a_value, b_value); }
        
        // Forget ourselves so that our drop() does not run (as our drop() attempts to free the descriptor)
        forget(self);
        // TODO: Create reference
        descriptor.rc_b.fetch_add(1, Ordering::Acquire);
        descriptor.rc_a.fetch_add(1, Ordering::Acquire);
        ()
    }
}
impl<T,A,B> Drop for DescriptorInitialiser<'_,T,A,B> {
    fn drop(&mut self){
        // SAFETY: As we are the only reference to this descriptor (during initialisation), we can be sure
        // that rc_a is 1, and that no other references have been taken (as rc_a is 1).
        unsafe {
            // Clear slot B first, as 
            self.0._clear_slot_b();
            self.0._clear();
        }
    }
}