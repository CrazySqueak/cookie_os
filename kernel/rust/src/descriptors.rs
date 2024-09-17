use core::sync::atomic::{AtomicU16,AtomicU64,Ordering,AtomicPtr};
use alloc::sync::Arc;
use core::ops::Drop;
use core::cell::SyncUnsafeCell;
use core::{mem,ptr};
use core::default::Default;
use alloc::boxed::Box;

pub type DescriptorID = u64;
pub type AtomicDescriptorID = AtomicU64;

pub enum DescriptorAcquireError {
    /// Descriptor is reserved, either being constructed or destructed
    DescriptorReserved,
    /// A B-Reference cannot be acquired because the b-slot is not active (rc_b is 0)
    BSlotNotAvailable,
    /// Descriptor was not found
    NotFound,
}

/** Descriptors are objects with are opened and stored in a descriptor table.
    These are a lot of kernel managed objects, including things such as processes, open files, and more.
    This struct also represents the "slot" itself in the table, and may be cleared and then re-used for any number of descriptors as applicable.
    
    A "handle" is a counted reference to a descriptor.
    There are two types of handles that may be opened: A-handles and B-handles. A-handles only count for rc_a, whereas B-handles count for both rc_a and rc_b.
    The semantic difference between the two depends on the descriptor, though A-handles can only access slots T and A, whereas B-handles can access slots T, A, and B.
    
    Each descriptor contains three descriptor-specific sections: the table section, the A section, and the B section.
    Once no more B-handles exist, the B section is dropped. Once no more handles exist, the A section is also dropped.
    The table section is never "dropped", but is overwritten when a new descriptor is assigned to that slot in the table.
    
    The rc_a count may be one of the following values: 0 - free, 1 - reserved (no other handles may be taken), 2+ - initialised
    The rc_b value is either 0 - deallocated, or 1+ - allocated. This is because there is no need for a "free" value in rc_b to signify that the descriptor is no longer in use, as that responsibility is handled by rc_a.
    Note: Once all B-handles are dropped, no more can be created.
*/
pub struct Descriptor<T,A,B> where T: Default {
    // Ref counts
    rc_a: AtomicU16,
    rc_b: AtomicU16,
    
    // ID
    id: AtomicDescriptorID,
    
    // Sections
    // Null pointers mean the value there has been dropped
    slot_t: T,
    slot_a: SyncUnsafeCell<Option<A>>,
    slot_b: SyncUnsafeCell<Option<B>>,
}
impl<T,A,B> Descriptor<T,A,B> where T: Default {
    /* Drop the slot_a value if non-null, then decrement rc_a to 0, marking this descriptor as free to be overwritten.
        SAFETY: rc_a should be 1. This MUST be the only thread trying to access this descriptor (enforced by rc_a being 1).
                rc_b must be 0 and slot_b must be freed. slot_t must not be in use (lifetimes should be pinned to the handle, so they do not outlive their A-handles).
                When clearing the entire descriptor, you should clear slot B (_clear_slot_b) first, then call _clear. */
    #[inline]
    unsafe fn _clear(&self){
        // Drop slot_a if initialised
        let slot_a = &mut *self.slot_a.get();
        slot_a.take();  // sets slot_a to None and drops the previous value
        
        // Now that slot_a is cleared, we decrement rc_a to 0. The descriptor is now free to be overwritten.
        self.rc_a.store(0, Ordering::Release);
    }
    /* Drop the slot_b value if non-null.
        SAFETY: rc_b should be 0. This MUST be the only thread trying to access this descriptor (enforced by rc_a being 1). */
    #[inline]
    unsafe fn _clear_slot_b(&self){
        // Drop slot_b if initialised
        let slot_b = &mut *self.slot_b.get();
        slot_b.take();  // sets slot_b to None and drops the previous value
    }
    
    /* Decrement rc_a, and perform destruction if applicable. */
    #[inline]
    unsafe fn _decrement_rc_a(&self){
        let prev_a = self.rc_a.fetch_sub(1, Ordering::SeqCst);
        if prev_a == 2 {
            // rc_a is now 1. Therefore, we are responsible for destroying slot A and clearing the descriptor
            self._clear();
        }
    }
    /* Decrement rc_b, and perform destruction if applicable.
        If decrementing both, then this must be called before decrement_rc_a. */
    #[inline]
    unsafe fn _decrement_rc_b(&self){
        let prev_b = self.rc_b.fetch_sub(1, Ordering::SeqCst);
        if prev_b == 1 {
            // rc_b is now 0. Therefore, we are responsible for destroying slot B
            self._clear_slot_b();
        }
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
    
    /* Create a new empty descriptor */
    fn new_empty() -> Self {
        Self {
            rc_a: AtomicU16::new(0),
            rc_b: AtomicU16::new(0),
            
            id: AtomicDescriptorID::new(0),
            
            slot_t: T::default(),
            slot_a: SyncUnsafeCell::new(None),
            slot_b: SyncUnsafeCell::new(None),
        }
    }
    
    /* Reserve the descriptor for use.
       This will increment rc_a from 0 (free) to 1 (reserved).
       Returns None if the operation failed (e.g. because the descriptor is already in use). */
    #[inline]
    fn reserve(&self, id: DescriptorID) -> Option<DescriptorInitialiser<T,A,B>> {
        // Attempt to begin initialisation by compare_exchange-ing the rc_a value.
        let r = self.rc_a.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed);
        if let Err(_) = r { return None; }  // If the compare_exchange failed, then the descriptor is already in use, so we return None.
        self.rc_b.store(0, Ordering::Relaxed);  // clear rc_B count
        self.id.store(id, Ordering::Relaxed);  // save the descriptor ID
        // rc_a is now equal to 1 (reserved). This therefore signifies that we are the only one currently using it, as all attempts to use it will now fail.
        Some(DescriptorInitialiser(self))
    }
    
    /* Acquire a new handle to the descriptor, if possible. */
    fn acquire_ref<const IS_B_REF: bool>(&self) -> Result<DescriptorHandle<T,A,B,IS_B_REF>,DescriptorAcquireError> {
        let rca_result = self.rc_a.fetch_update(Ordering::Acquire, Ordering::Acquire, |rca| if rca >= 2 { Some(rca+1) } else { None });  // If rc_a >= 2, increment rc and continue. Otherwise, fail (cannot reference 1 as it's initialising, cannot reference 0 as it's not present).
        if let Err(_) = rca_result { return Err(DescriptorAcquireError::DescriptorReserved) };
        
        if IS_B_REF {
            let rcb_result = self.rc_b.fetch_update(Ordering::Acquire, Ordering::Acquire, |rcb| if rcb >= 1 { Some(rcb+1) } else { None });
            if let Err(_) = rcb_result {
                // Failed: rc_b is 0 (so b is unavailable)
                // We must first decrement rc_a as we had incremented it previously
                unsafe { self._decrement_rc_a() };
                // And then return an error
                return Err(DescriptorAcquireError::BSlotNotAvailable);
            }
        }
        
        // We have now incremented rc_a, as well as rc_b (if applicable).
        // Return a handle
        Ok(DescriptorHandle(self))
    }
}

/* An RAII guard used for initialising a descriptor.
    If commit() is called, the descriptor's rc_a will be incremented to 2, and its descriptor will become ready for use.
    If this is otherwise dropped, the descriptor will be cleared and have its rc_a decremented back to 0 (free). */
pub struct DescriptorInitialiser<'r,T,A,B>(&'r Descriptor<T,A,B>) where T: Default;
impl<'r,T,A,B> DescriptorInitialiser<'r,T,A,B> where T: Default {
    #[inline]
    pub fn id(&self) -> u64 { self.0.id.load(Ordering::Relaxed) }
    #[inline]
    pub fn slot_t<'a>(&'a self) -> &'a T { &self.0.slot_t }
    
    /* Finish the initialisation of the descriptor, putting a_value into slot a, b_value into slot b, and eventually incrementing its rc_a count to 2, its rc_b count to 1, and returning a B-handle.
        Once this method is run, the descriptor may have any number of references taken in the future, and we no longer exclusively own it. */
    pub fn commit(self, a_value: A, b_value: B) -> DescriptorHandleB<'r,T,A,B> {
        let descriptor = self.0;
        // SAFETY: As we are the only handle to the descriptor (it is still being initialised), we can ensure that
        // the required safety invariants are met.
        unsafe { descriptor._init_slots(a_value, b_value); }
        
        // Forget ourselves so that our drop() does not run (as our drop() attempts to free the descriptor)
        mem::forget(self);
        // Create handle
        descriptor.rc_b.fetch_add(1, Ordering::Acquire);
        descriptor.rc_a.fetch_add(1, Ordering::Acquire);
        DescriptorHandle(descriptor)
    }
}
impl<T,A,B> Drop for DescriptorInitialiser<'_,T,A,B> where T: Default {
    fn drop(&mut self){
        // SAFETY: As we are the only handle to this descriptor (during initialisation), we can be sure
        // that rc_a is 1, and that no other references have been taken (as rc_a is 1).
        unsafe {
            // Clear slot B first, as 
            self.0._clear_slot_b();
            self.0._clear();
        }
    }
}

pub struct DescriptorHandle<'r,T,A,B,const IS_B_REF: bool>(&'r Descriptor<T,A,B>) where T: Default;
pub type DescriptorHandleA<'r,T,A,B> = DescriptorHandle<'r,T,A,B,false>;
pub type DescriptorHandleB<'r,T,A,B> = DescriptorHandle<'r,T,A,B,true>;
impl<'r,T,A,B,const IS_B_REF: bool> DescriptorHandle<'r,T,A,B,IS_B_REF> where T: Default {
    /* Get the ID of the descriptor. */
    #[inline]
    pub fn get_id(&self) -> DescriptorID {
        self.0.id.load(Ordering::Relaxed)
    }
    
    /* Get a reference to the slot_t data for this descriptor.
        This is bound by the lifetime of the DescriptorHandle so that it only applies to the requested descriptor.
        If you want one that lives as long as the table itself (instead of only the given allocation of the slot), use get_t_forever. */
    #[inline]
    pub fn get_t<'a>(&'a self) -> &'a T {
        &self.0.slot_t
    }
    /* This reference to T will live as long as the slot itself, even if it is freed and then re-used for a different descriptor. */
    #[inline]
    pub fn get_t_forever(&self) -> &'r T {
        &self.0.slot_t
    }
    
    /* Get a reference to the A-slot in the descriptor.
        Note: it is impossible to mutate the A-slot itself in this state. Please use interior mutability if mutation is required. */
    #[inline]
    pub fn get_a<'a>(&'a self) -> &'a A {
        // SAFETY: Since this A-ref exists, rc_a is >= 2 and will not decrease below that as long as this A-ref is not dropped
        //          Since rc_a is >= 2, A will not be borrowed mutably by the destructor/initialiser (and it cannot be borrowed mutably in any other way).
        let cellref = unsafe { &*self.0.slot_a.get() };
        cellref.as_ref().unwrap()
    }
    
    /* Create another A-handle using this handle.
        Since an existing handle is present and in-scope, this operation is guaranteed to succeed (and is quicker than acquire_ref as it can skip several checks). */
    pub fn clone_a_ref(&self) -> DescriptorHandleA<'r,T,A,B> {
        // Increment ref count
        self.0.rc_a.fetch_add(1, Ordering::Acquire);
        // Create reference
        DescriptorHandle(self.0)
    }
}
impl<'r,T,A,B> DescriptorHandleA<'r,T,A,B> where T: Default {
    /* Attempt to upgrade this A-handle to a B-handle. */
    pub fn upgrade(self) -> Result<DescriptorHandleB<'r,T,A,B>,DescriptorAcquireError> {
        // Currently this just calls .acquire_ref, but in theory it could be replaced with a more optimised version that takes advantage of the fact that a ref already exists
        self.0.acquire_ref::<true>()
    }
}
impl<'r,T,A,B> DescriptorHandleB<'r,T,A,B> where T: Default {
    /* Get a reference to the B-slot in the descriptor.
        Note: it is impossible to mutate the B-slot itself in this state. Please use interior mutability if mutation is required. */
    #[inline]
    pub fn get_b<'a>(&'a self) -> &'a B {
        // SAFETY: Since this B-ref exists, rc_b is >= 1 and will not decrease below that as long as this B-ref is not dropped
        //          Since rc_b is >= 2, B will not be borrowed mutably by the destructor/initialiser (and it cannot be borrowed mutably in any other way).
        let cellref = unsafe { &*self.0.slot_b.get() };
        cellref.as_ref().unwrap()
    }
    
    /* Create another B-handle using this handle.
        Since an existing B-handle is present and in-scope, this operation is guaranteed to succeed (and is quicker than acquire_ref as it can skip several checks). */
    pub fn clone_b_ref(&self) -> DescriptorHandleB<'r,T,A,B> {
        // Increment ref counts
        self.0.rc_a.fetch_add(1, Ordering::Acquire);
        self.0.rc_b.fetch_add(1, Ordering::Acquire);
        // Create handle
        DescriptorHandle(self.0)
    }
    
    /* Attempt to downgrade this B-handle to an A-handle. */
    pub fn downgrade(self) -> DescriptorHandleA<'r,T,A,B> {
        // Currently this just calls .clone_a_ref, but in theory it could be replaced with a more optimised version that takes advantage of the fact that a ref already exists
        self.clone_a_ref()
    }
}

impl<T,A,B,const IS_B_REF: bool> Drop for DescriptorHandle<'_,T,A,B,IS_B_REF> where T: Default {
    fn drop(&mut self){
        let descriptor = self.0;
        // Drop B reference
        if IS_B_REF {
            unsafe { descriptor._decrement_rc_b(); }
        }
        // Drop A reference
        unsafe { descriptor._decrement_rc_a(); }
    }
}

/* A table of descriptors. N = number per table, M = number of different sub-tables (see below)

Sub Tables:
When the current table runs out of space, it will allocate one or more sub-tables (should be power of two for best performance).
To avoid linked-list level performance, the subtable of index ID%M is used. (if that subtable is full, then index (ID/M)%M is used, and so on.
Sub-tables may not be dropped unless their parent is also dropped, as it is expected that a subtable, once allocated, will remain allocated.
N and M are "voodoo constants", whose values are chosen by luck and intuition. */
pub struct DescriptorTable<T,A,B, const N: usize, const M: usize> where T: Default {
    next_id: AtomicDescriptorID,
    table: DescriptorTableInner<T,A,B,N,M>,
}
impl<T,A,B, const N: usize, const M: usize> DescriptorTable<T,A,B,N,M> where T: Default {
    #[inline]
    pub fn new() -> Self {
        Self {
            next_id: AtomicDescriptorID::new(1),  // ID 0 is not used as it would confuse people
            table: DescriptorTableInner::new(),
        }
    }
    
    /* Get a handle to the descriptor with the given ID, or an error if it could not be done. */
    fn acquire<const IS_B_REF: bool>(&self, id: DescriptorID) -> Result<DescriptorHandle<T,A,B,IS_B_REF>,DescriptorAcquireError> {
        self.table.acquire::<IS_B_REF>(id)
    }
    /* Get an A-handle to the descriptor with the given ID, or an error if it could not be done. */
    pub fn acquire_a(&self, id: DescriptorID) -> Result<DescriptorHandleA<T,A,B>,DescriptorAcquireError> {
        self.acquire::<false>(id)
    }
    /* Get a B-handle to the descriptor with the given ID, or an error if it could not be done. */
    pub fn acquire_b(&self, id: DescriptorID) -> Result<DescriptorHandleB<T,A,B>,DescriptorAcquireError> {
        self.acquire::<true>(id)
    }
    /* Create a new descriptor, and return the initialiser, allowing you to initialise slots T, A, and B as necessary before commit()-ing it and opening the descriptor for regular use. */
    pub fn create_new_descriptor(&self) -> DescriptorInitialiser<T,A,B> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.table.allocate_empty(id)
    }
}

/* Descriptor tables are multi-level, however some extra stuff has to be stored with the root. This is the multi-level insides or whatever. You know what i mean */
pub struct DescriptorTableInner<T,A,B, const N: usize, const M: usize> where T: Default {
    descriptors: [Descriptor<T,A,B>; N],
    subtables: [AtomicPtr<Self>; M],
}
impl<T,A,B, const N: usize, const M: usize> DescriptorTableInner<T,A,B,N,M> where T: Default {
    #[inline]
    pub fn new() -> Self {
        Self {
            descriptors: core::array::from_fn(|i| Descriptor::new_empty()),
            subtables: [const { AtomicPtr::new(ptr::null_mut()) }; M],
        }
    }
    
    /* Return the subtable in that slot, or create a new one if one is not already there. */
    #[inline]
    fn _get_or_create_sub_table(&self, idx: usize) -> &Self {
        // Optimisation: Only allocate a new subtable if there isn't a sub-table already there
        // we still have to do a compare_exchange if there isn't as otherwise a sub-table could be put there while our back is turned,
        // but it means we don't have to allocate and de-allocate a boxed subtable for every single subtable lookup.
        let st_pointer = if self.subtables[idx].load(Ordering::Relaxed) == ptr::null_mut() {
            let new_subtable_ptr = Box::into_raw(Box::new(Self::new()));
            match self.subtables[idx].compare_exchange(ptr::null_mut(), new_subtable_ptr, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => new_subtable_ptr,  // All ok
                Err(existing_ptr) => {
                    // Something is already there!
                    // Drop our original subtable as it's no longer needed
                    drop(unsafe{ Box::from_raw(new_subtable_ptr) });
                    // And return the existing one
                    existing_ptr
                }
            }
        } else { self.subtables[idx].load(Ordering::Relaxed) };
        
        // Borrow an immutable reference to the chosen subtable
        // SAFETY: We've already checked for null pointers above (technically twice - once to skip allocating if not needed, and a second time to assign the new table)
        //          st_pointer MUST always be valid, and subtables MUST NOT be freed. If it wasn't for the atomicity required, I'd be using a regular Box<>.
        //          So, treat it like a Box<> which is owned by this table and you can't really go wrong.
        unsafe { &*st_pointer }
    }
    
    // a version of _get_or_create_sub_table that returns None if none was found instead of creating a new one
    #[inline]
    fn _get_sub_table_or_none(&self, idx: usize) -> Option<&Self> {
        let ptr = self.subtables[idx].load(Ordering::Relaxed);
        if ptr == ptr::null_mut() { None }
        else { Some(unsafe { &*ptr }) }
    }
    
    /* Find the descriptor with the given ID.
        No guarantee is made that the descriptor will still be extant and correct when you go to use it.
        The only guarantee is that the descriptor was there at some point (as descriptor IDs are not re-used and descriptors cannot be moved, the descriptor being non-extant or overwritten implies that it is gone forever. */
    fn _search(&self, id: DescriptorID, st_indexer: usize) -> Option<&Descriptor<T,A,B>> {
        // Find descriptor
        for descriptor in &self.descriptors {
            if descriptor.id.load(Ordering::Acquire) == id { return Some(descriptor); }
        }
        // Find sub-table
        let st_index = st_indexer%M;
        if let Some(subtable) = self._get_sub_table_or_none(st_index) {
            subtable._search(id, st_indexer/M)
        } else { None }
    }
    /* Get a handle to the descriptor with the given ID, or an error if it could not be done. */
    #[inline]
    pub fn acquire<const IS_B_REF: bool>(&self, id: DescriptorID) -> Result<DescriptorHandle<T,A,B,IS_B_REF>,DescriptorAcquireError> {
        // Locate the descriptor and acquire a handle
        let descriptor = self._search(id, id.try_into().unwrap()).ok_or(DescriptorAcquireError::NotFound)?;
        let desc_ref = descriptor.acquire_ref::<IS_B_REF>()?;
        
        // Now that we have a handle, the descriptor will not be erased or replaced
        // Check the ID to ensure it wasn't overwritten between finding the descriptor and acquiring the handle
        if desc_ref.get_id() != id { return Err(DescriptorAcquireError::NotFound); }  // overwritten
        // All ok
        Ok(desc_ref)
    }
    
    fn _allocate_empty(&self, id: DescriptorID, st_indexer: usize) -> DescriptorInitialiser<T,A,B> {
        // Find an empty slot
        for descriptor in &self.descriptors {
            if let Some(desc) = descriptor.reserve(id) { return desc; }  // we got it!
        }
        // Find a sub-table and reserve in there
        let st_index = st_indexer%M;
        self._get_or_create_sub_table(st_index)._allocate_empty(id, st_indexer/M)
    }
    /* Allocate an empty slot for a new descriptor with the given ID, returning the initialiser which can be used to initialise it.
        Warning: If a descriptor with that ID already exists in the table, then it is undefined which one is returned by methods such as acquire. */
    #[inline]
    pub fn allocate_empty(&self, id: DescriptorID) -> DescriptorInitialiser<T,A,B> {
        self._allocate_empty(id, id.try_into().unwrap())
    }
}
impl<T,A,B, const N: usize, const M: usize> Drop for DescriptorTableInner<T,A,B,N,M> where T: Default {
    fn drop(&mut self){
        // Drop sub-tables (as they're stored as pointers and wouldn't be dropped otherwise)
        for st_ptr in &mut self.subtables {
            let ptr = st_ptr.get_mut();
            let st = core::mem::replace(ptr, ptr::null_mut());
            if st != ptr::null_mut() { drop(unsafe{ Box::from_raw(st) }) };
        }
    }
}