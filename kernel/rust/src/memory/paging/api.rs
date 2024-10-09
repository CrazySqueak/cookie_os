
use core::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::sync::hspin::{HRwLock as RwLock, HRwLockReadGuard as RwLockReadGuard, HRwLockWriteGuard as RwLockWriteGuard, HRwLockUpgradableGuard as RwLockUpgradableGuard, HMutex, HMutexGuard};  // we're now using HLocks because that's what we always should've been using
use crate::multitasking::cpulocal::CpuLocal;

use super::*;

type BaseTLPageAllocator = arch::TopLevelPageAllocator;
use arch::{set_active_page_table};

// Page-Alignable Numbers
/// The alignment (in bytes) for pages. In other words, the minimum possible amount of memory worth caring about for system-wide memory management.
pub const PAGE_ALIGN: usize = super::MIN_PAGE_SIZE;
use core::num::NonZeroUsize;

#[deprecated]
pub type PageAlignedUsize = PageAllocationSizeT;
pub trait PageAlignedValue<Wraps>: Sized + Copy {
    /// Panics if an invalid value is provided.
    #[track_caller]
    fn new(x: Wraps) -> Self;
    /// May cause undefined behaviour if an invalid value is provided
    unsafe fn new_unchecked(x: Wraps) -> Self;
    /// Returns Some() if valid, None if invalid.
    fn new_checked(x: Wraps) -> Option<Self>;
    /// Rounds to the next valid value, returning both the new value and the amount rounded by. (round up for sizes, down for offsets)
    fn new_rounded_with_excess(x: Wraps) -> (Self,usize);
    /// Rounds to the next valid value (round up for sizes, down for offsets)
    fn new_rounded(x: Wraps) -> Self {
        Self::new_rounded_with_excess(x).0
    }

    /// Get the contained value
    fn get(self) -> Wraps;
}

macro_rules! ftorawiiwctc {  // I'm sure you can guess what this stands for.
    ($T:ident, $Wraps:ident) => {
        impl core::convert::From<$T> for $Wraps {
            fn from(value: $T) -> $Wraps {
                <$T as PageAlignedValue<$Wraps>>::get(value)
            }
        }
        impl core::convert::TryFrom<$Wraps> for $T {
            type Error = ();
            fn try_from(value: $Wraps) -> Result<$T,()> {
                <$T as PageAlignedValue<$Wraps>>::new_checked(value).ok_or(())
            }
        }

        impl core::fmt::LowerHex for $T {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::LowerHex::fmt(&<$T as PageAlignedValue<$Wraps>>::get(*self),f)
            }
        }
        impl core::fmt::UpperHex for $T {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::UpperHex::fmt(&<$T as PageAlignedValue<$Wraps>>::get(*self),f)
            }
        }
        impl core::fmt::Octal for $T {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Octal::fmt(&<$T as PageAlignedValue<$Wraps>>::get(*self),f)
            }
        }
        impl core::fmt::Binary for $T {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Binary::fmt(&<$T as PageAlignedValue<$Wraps>>::get(*self),f)
            }
        }
        impl core::fmt::Display for $T {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(&<$T as PageAlignedValue<$Wraps>>::get(*self),f)
            }
        }
    }
}

#[repr(transparent)]
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord,Hash,Debug)]
/// A non-zero, page-aligned value, suitable for specifying the size of page allocations. Rounds up.
pub struct PageAllocationSizeT(NonZeroUsize);
impl PageAlignedValue<usize> for PageAllocationSizeT {
    /// Returns self if page-aligned and non-zero. Otherwise, panics (in debug builds). If debug assertions are disabled and these conditions are not met, behaviour is undefined.
    fn new(x: usize) -> Self {
        Self::new_const(x)  // const_trait_impl doesn't seem to be going anywhere yet...
    }
    unsafe fn new_unchecked(x: usize) -> Self {
        Self::new_unchecked_const(x)
    }
    /// Returns Some() if page-aligned and non-zero. Otherwise, returns None.
    fn new_checked(x: usize) -> Option<Self> {
        if x == 0 { None }
        else if x%PAGE_ALIGN == 0 { Some(Self::new(x)) }
        else { None }
    }
    /// Round up to the next non-zero, page-aligned value, and return both the rounded value and the amount added to do this.
    /// In other words, where the input is x and the output is (y,rem): y = x+rem
    fn new_rounded_with_excess(x: usize) -> (Self,usize) {
        if x == 0 { (Self(unsafe{NonZeroUsize::new_unchecked(PAGE_ALIGN)}), PAGE_ALIGN) }  // safety: PAGE_ALIGN is never zero
        else if x%PAGE_ALIGN == 0 { (Self::new(x), 0) }
        else {
            // Round up since this is a size
            let excess = PAGE_ALIGN-(x%PAGE_ALIGN);
            (Self::new(x+excess), excess)
        }
    }

    /// Get the stored integer
    fn get(self) -> usize {
        self.get_const()
    }
}
impl PageAllocationSizeT {
    pub const fn new_const(x: usize) -> Self {
        assert!(x != 0, "PageAllocationSizeT must be non-zero");  // Ensure we don't cause UB
        debug_assert!(x%PAGE_ALIGN == 0, "PageAllocationSizeT must be page-aligned.");
        unsafe{Self::new_unchecked_const(x)}
    }
    pub const unsafe fn new_unchecked_const(x: usize) -> Self {
        Self(NonZeroUsize::new_unchecked(x))
    }

    pub const fn get_const(self) -> usize {
        let x = self.0.get();
        debug_assert!(x%PAGE_ALIGN==0);  // this is already checked for when setting it, but for safety's sake let's check it before returning it as well
        debug_assert!(x != 0);
        x
    }
    /// Get the stored integer as a NonZeroUsize
    pub const fn get_nz(self) -> NonZeroUsize {
        let x = self.0;
        debug_assert!(x.get()%PAGE_ALIGN==0);  // this is already checked for when setting it, but for safety's sake let's check it before returning it as well
        x
    }
}
ftorawiiwctc!(PageAllocationSizeT, usize);
impl core::convert::From<PageAllocationSizeT> for NonZeroUsize {
    fn from(value: PageAllocationSizeT) -> NonZeroUsize {
        value.get_nz()
    }
}
impl core::convert::TryFrom<NonZeroUsize> for PageAllocationSizeT {
    type Error = ();
    fn try_from(value: NonZeroUsize) -> Result<PageAllocationSizeT,()> {
        PageAllocationSizeT::new_checked(value.get()).ok_or(())
    }
}

#[repr(transparent)]
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord,Hash,Debug)]
/// A page-aligned, signed value, suitable for specifying offsets in page-aligned increments. Rounds down.
pub struct PageAlignedOffsetT(isize);
impl PageAlignedValue<isize> for PageAlignedOffsetT {
    fn new(x: isize) -> Self {
        Self::new_const(x)
    }
    unsafe fn new_unchecked(x: isize) -> Self {
        Self::new_unchecked_const(x)
    }
    fn new_checked(x: isize) -> Option<Self> {
        if x.rem_euclid(PAGE_ALIGN as isize) == 0 { Some(Self::new(x)) }
        else { None }
    }

    fn new_rounded_with_excess(x: isize) -> (Self,usize) {
        if x.rem_euclid(PAGE_ALIGN as isize) == 0 { (Self::new(x), 0) }
        else {
            // Round down since this is an offset
            let excess = x.rem_euclid(PAGE_ALIGN as isize);
            (Self::new(x-excess), excess as usize)
        }
    }

    fn get(self) -> isize {
        self.get_const()
    }
}
impl PageAlignedOffsetT {
    pub const fn new_const(x: isize) -> Self {
        debug_assert!(x.rem_euclid(PAGE_ALIGN as isize) == 0, "PageAlignedOffsetT must be page-aligned.");
        unsafe{Self::new_unchecked_const(x)}
    }
    pub const unsafe fn new_unchecked_const(x: isize) -> Self {
        Self(x)
    }

    pub const fn get_const(self) -> isize {
        let x = self.0;
        debug_assert!(x.rem_euclid(PAGE_ALIGN as isize)==0);  // this is already checked for when setting it, but for safety's sake let's check it before returning it as well
        x
    }
}
ftorawiiwctc!(PageAlignedOffsetT, isize);
impl core::ops::Add for PageAlignedOffsetT {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.0 + rhs.0)
    }
}
impl core::ops::Sub for PageAlignedOffsetT {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.0 - rhs.0)
    }
}

#[repr(transparent)]
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord,Hash,Debug)]
/// A page-aligned, unsigned value, suitable for specifying virtual addresses at page boundraries. Rounds down.
pub struct PageAlignedAddressT(usize);
impl PageAlignedValue<usize> for PageAlignedAddressT {
    fn new(x: usize) -> Self {
        Self::new_const(x)
    }
    unsafe fn new_unchecked(x: usize) -> Self {
        Self::new_unchecked_const(x)
    }
    fn new_checked(x: usize) -> Option<Self> {
        if x%PAGE_ALIGN == 0 { Some(Self::new(x)) }
        else { None }
    }

    fn new_rounded_with_excess(x: usize) -> (Self,usize) {
        if x%PAGE_ALIGN == 0 { (Self::new(x), 0) }
        else {
            // Round down since this is an offset
            let excess = x%PAGE_ALIGN;
            (Self::new(x-excess), excess as usize)
        }
    }

    fn get(self) -> usize {
        self.get_const()
    }
}
impl PageAlignedAddressT {
    pub const fn new_const(x: usize) -> Self {
        debug_assert!(x%PAGE_ALIGN == 0, "PageAlignedAddressT must be page-aligned.");
        unsafe{Self::new_unchecked_const(x)}
    }
    pub const unsafe fn new_unchecked_const(x: usize) -> Self {
        Self(x)
    }

    pub const fn get_const(self) -> usize {
        let x = self.0;
        debug_assert!(x%PAGE_ALIGN==0);  // this is already checked for when setting it, but for safety's sake let's check it before returning it as well
        x
    }
}
ftorawiiwctc!(PageAlignedAddressT, usize);

// FLAGS & STUFF
#[derive(Debug,Clone,Copy)]
pub struct PageFlags {
    pub tflags: TransitivePageFlags,
    pub mflags: MappingSpecificPageFlags,
}
impl PageFlags {
    pub fn new(tflags: TransitivePageFlags, mflags: MappingSpecificPageFlags) -> Self {
        Self { tflags, mflags }
    }
    pub fn empty() -> Self {
        Self::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty())
    }
}
macro_rules! impl_pf_part_ops {
    ($rhstype:ty, $pname:ident, $tn:ident, $mn:ident) => {
        #[automatically_derived]
        impl core::ops::BitAnd<$rhstype> for PageFlags {
            type Output = $rhstype;
            fn bitand(self, rhs: $rhstype) -> Self::Output {
                let Self { tflags: $tn, mflags: $mn } = self;
                let mut $pname = $pname; $pname &= rhs;
                $pname
            }
        }

        #[automatically_derived]
        impl core::ops::BitOr<$rhstype> for PageFlags {
            type Output = Self;
            fn bitor(self, rhs: $rhstype) -> Self::Output {
                let Self { tflags: $tn, mflags: $mn } = self;
                let mut $pname = $pname; $pname |= rhs;
                Self { tflags: $tn, mflags: $mn }
            }
        }
        #[automatically_derived]
        impl core::ops::BitOrAssign<$rhstype> for PageFlags {
            fn bitor_assign(&mut self, rhs: $rhstype) {
                self.$pname |= rhs;
            }
        }

        #[automatically_derived]
        impl core::ops::BitXor<$rhstype> for PageFlags {
            type Output = Self;
            fn bitxor(self, rhs: $rhstype) -> Self::Output {
                let Self { tflags: $tn, mflags: $mn } = self;
                let mut $pname = $pname; $pname ^= rhs;
                Self { tflags: $tn, mflags: $mn }
            }
        }
        #[automatically_derived]
        impl core::ops::BitXorAssign<$rhstype> for PageFlags {
            fn bitxor_assign(&mut self, rhs: $rhstype) {
                self.$pname ^= rhs;
            }
        }

        #[automatically_derived]
        impl core::ops::Sub<$rhstype> for PageFlags {
            type Output = Self;
            fn sub(self, rhs: $rhstype) -> Self::Output {
                let Self { tflags: $tn, mflags: $mn } = self;
                let mut $pname = $pname; $pname -= rhs;
                Self { tflags: $tn, mflags: $mn }
            }
        }
        #[automatically_derived]
        impl core::ops::SubAssign<$rhstype> for PageFlags {
            fn sub_assign(&mut self, rhs: $rhstype) {
                self.$pname -= rhs;
            }
        }
    }
}
impl_pf_part_ops!(TransitivePageFlags, tflags, tflags, mflags);
impl_pf_part_ops!(MappingSpecificPageFlags, mflags, tflags, mflags);
macro_rules! impl_pf_compound_ops {
    (binop $traitname:path, $mname:ident) => {
        impl $traitname for PageFlags {
            type Output = Self;
            fn $mname(self, rhs: Self) -> Self::Output {
                use $traitname;
                let Self { tflags, mflags } = self;
                let tflags = tflags.$mname(rhs.tflags);
                let mflags = mflags.$mname(rhs.mflags);
                Self { tflags, mflags }
            }
        }
    };
    (assign $traitname:path, $mname:ident) => {
        impl $traitname for PageFlags {
            fn $mname(&mut self, rhs: Self) {
                use $traitname;
                self.tflags.$mname(rhs.tflags);
                self.mflags.$mname(rhs.mflags);
            }
        }
    };
}
impl_pf_compound_ops!(binop core::ops::BitAnd, bitand);
impl_pf_compound_ops!(assign core::ops::BitAndAssign, bitand_assign);
impl_pf_compound_ops!(binop core::ops::BitOr, bitor);
impl_pf_compound_ops!(assign core::ops::BitOrAssign, bitor_assign);
impl_pf_compound_ops!(binop core::ops::BitXor, bitxor);
impl_pf_compound_ops!(assign core::ops::BitXorAssign, bitxor_assign);
impl_pf_compound_ops!(binop core::ops::Sub, sub);
impl_pf_compound_ops!(assign core::ops::SubAssign, sub_assign);
bitflags::bitflags! {
    // These flags follow a "union" pattern - flags applied to upper levels will also override lower levels (the most restrictive version winning)
    // Therefore: the combination of all flags should be the most permissive/compatible option
    #[derive(Debug,Clone,Copy)]
    pub struct TransitivePageFlags: u16 {
        // User can access this page.
        const USER_READABLE = 1<<0;
        // This page can be written to. (both by the kernel, and by the user if they have access to it)
        const WRITEABLE = 1<<1;
        #[deprecated]
        const USER_WRITEABLE = 1<<1;  // old alias for WRITEABLE
        // Execution is allowed. (if feature per_page_NXE_bit is not enabled then this is ignored)
        const EXECUTABLE = 1<<2;
    }
    // These flags are non-transitive. For regular mappings, they work the same as transitive flags. For sub-table mappings, they apply to the sub-table itself, rather than any descendant page mappings.
    #[derive(Debug,Clone,Copy)]
    pub struct MappingSpecificPageFlags: u16 {
        // Page mapping is global (available in all address spaces), and should not be flushed from the TLB on an address-space switch.
        // On pages: Page is not flushed from the TLB on address-space switch.
        // On sub-tables: Architecture dependent.
        const GLOBAL = 1<<0;
        // Page is pinned. Pinned pages must not be moved in (or swapped out of) physical memory, as they may contain important structures such as page tables or interrupt handlers.
        // This is a custom flag which is interpreted by the OS' page handler
        // On pages: Page must always point to the same frame. Frame's contents must not be moved.
        // On sub-tables: No effect.
        // #[deprecated]
        const PINNED = 1<<1;
        // Affects the "memory type" of the selected area - idk what this does because what I've read is very vague
        // On pages: influences the memory type of the memory mapped via the page
        // On sub-tables: influences the memory type used for reading the sub-table
        const CACHE_DISABLE = 1<<2;
        // Affects the "memory type" of the selected area - enables "write-through caching", meaning that writes occur immediately instead of being cached and then written back later
        // On pages: influences the memory type of the memory mapped via the page
        // On sub-tables: influences the memory type used for reading the sub-table
        const CACHE_WRITE_THROUGH = 1<<3;
    }
}

macro_rules! pageFlags {
    ($($k:ident:$i:ident),*) => {{
        use $crate::memory::paging::{PageFlags,TransitivePageFlags as t,MappingSpecificPageFlags as m};
        PageFlags::empty() $(| $k::$i)*
    }}
}
pub(crate) use pageFlags;

// LOCKED PAGE ALLOCATOR
#[derive(Debug,Clone,Copy)]
pub struct LPAMetadata {
    // Offset in VMEM for the start of the page table's jurisdiction. For top-level tables this is 0. For tables nested inside of other tables, this might not be 0
    pub offset: usize,
    // Default options for new write guards
    pub default_options: LPAWGOptions,
}
struct LPAInternal<PFA: PageFrameAllocator> {
    /// The locked allocator
    lock: HMutex<PFA>,
    /// Other metadata
    meta: LPAMetadata,
    /// active_count: If non-zero, we must bother with TLB invalidation
    active_count: AtomicU16,
}
// pub const ACTIVE_ID_EMPTY: u8 = 0;
// pub const ACTIVE_ID_UNKNOWABLE: u8 = 255;
impl<PFA: PageFrameAllocator> LPAInternal<PFA> {
    fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self {
            lock: HMutex::new(alloc),
            meta: meta,
            active_count: AtomicU16::new(0),
        }
    }
    pub fn metadata(&self) -> LPAMetadata {
        self.meta
    }
}

pub struct LockedPageAllocator<PFA: PageFrameAllocator>(Arc<LPAInternal<PFA>>);
impl<PFA: PageFrameAllocator> LockedPageAllocator<PFA> {
    pub fn new(alloc: PFA, meta: LPAMetadata) -> Self {
        Self(Arc::new(LPAInternal::new(alloc, meta)))
    }
    
    pub fn clone_ref(x: &Self) -> Self {
        Self(Arc::clone(&x.0))
    }
    
    pub fn metadata(&self) -> &LPAMetadata {
        &(*self.0).meta
    }
    
    /* Lock the allocator for reading until _end_active is called.
    This is intended to be used when the page table is possibly being read/cached by the CPU, as when locked by _begin_active, writes via write_when_active are still possible (as they flush the TLB).
    This returns the lock guard to allow temporary access to the allocator, however: 1. the allocator must only be read from, and 2. the lock should be released (not leaked)s once activation is finished. */
    pub(super) fn _begin_active(&self) -> impl Deref<Target=PFA> + '_ {
        // Increment active_count
        self.0.active_count.fetch_add(1,Ordering::Acquire);

        // Lock temporarily
        // This ensures that we are not being written to by a writer that is unaware of our existence
        // (we immediately release the lock afterwards as anyone who locks for writing after us will check active_count,
        //      see that we're active, and issue the correct TLB flush commands as applicable)
        let lock = self.0.lock.lock();

        // Return
        lock
    }
    pub(super) unsafe fn _end_active(&self){
        // Decrement active_count
        self.0.active_count.fetch_sub(1,Ordering::Release);

        //core::sync::atomic::compiler_fence(Ordering::SeqCst);
    }
    
    /* Write to a page table that is currently active, provided there are no other read/write locks.
        Writes using this guard will automatically invalidate the TLB entries as needed.*/
    pub(super) fn write_when_active(&self) -> LPAWriteGuard<PFA> {
        // Obtain a lock guard
        // Once we hold a guard, we guarantee that no more readers will activate the page without us knowing
        let guard = self.0.lock.lock();

        let mut options = self.metadata().default_options.clone();
        options.auto_flush_tlb = self.0.active_count.load(Ordering::Relaxed) > 0;  // if we're active, flush the TLB
        // Note: this is safe because a page cannot become active before first incrementing active_count and then acquiring the lock
        // Thus, while we're writing, active_count cannot be incremented (there may be some chicanery implemented later on if necessary but active_count will never go from 0 -> 1 while we hold the lock)

        return LockedPageAllocatorWriteGuard { guard, allocator: Self::clone_ref(self), options };
    }
    
    pub fn allocate(&self, size: PageAllocationSizeT, alloc_strat: PageAllocationStrategies) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate(size, alloc_strat)
    }
    pub fn allocate_at(&self, addr: PageAlignedAddressT, size: PageAllocationSizeT) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate_at(addr, size)
    }
    /// Allocate page(s) dynamically such that the given physical address would be able to be mapped using a simple .set_addr (i.e. such that phys minus base would be page-aligned)
    pub fn allocate_alignedoffset(&self, size: usize, alloc_strat: PageAllocationStrategies, phys_addr: usize) -> Option<PageAllocation<PFA>> {
        self.write_when_active().allocate_alignedoffset(size, alloc_strat, phys_addr)
    }

    /// Get the physical address of the page table
    /// Intended for use as a heuristic only
    pub fn get_phys_addr(&self) -> usize {
        ptaddr_virt_to_phys(self.0.lock.lock().get_page_table_ptr() as usize)
    }
}

pub struct PagingContext(LockedPageAllocator<BaseTLPageAllocator>);
impl PagingContext {
    pub fn new() -> Self {
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Creating new paging context.");
        let mut allocator = BaseTLPageAllocator::new();
        // Add global pages
        for i in 0..global_pages::N_GLOBAL_TABLES {
            let phys_addr = global_pages::GLOBAL_TABLE_PHYSADDRS[i];
            let flags = global_pages::GLOBAL_TABLE_FLAGS[i];
            // SAFETY: See documentation for put_global_table and GLOBAL_TABLE_PHYSADDRS
            unsafe{ allocator.put_global_table(global_pages::GLOBAL_PAGES_START_IDX+i, phys_addr, flags); }
        }
        // Add kernel static global page
        {
            let phys_addr = *global_pages::KERNEL_STATIC_PHYSADDR;
            let flags = *global_pages::KERNEL_STATIC_FLAGS;
            unsafe{ allocator.put_global_table(global_pages::KERNEL_STATIC_PT_IDX, phys_addr, flags); }
        }
        // Return
        let dopts = LPAWGOptions::new_default();
        Self(LockedPageAllocator::new(allocator, LPAMetadata { offset: 0, default_options: dopts }))
    }
    pub fn clone_ref(x: &Self) -> Self {
        Self(LockedPageAllocator::clone_ref(&x.0))
    }
    
    /* Activate this page table. Once active, this page table will be used to map virtual addresses to physical ones.
        Use of Arc ensures that the page table will not be dropped if it is still active.
        DEADLOCK: If you have a write guard active in the current thread, this *WILL* deadlock.
        DANGER: You MUST follow the proper rules unless you want your program to CRASH UNEXPECTEDLY! This is serious shit!
         * The kernel stack should be at the same virtual memory address in both the old and new tables. It cannot simply be moved to a different one.
         * All heap objects and objects pointed to by pointers should be at the same virtual memory address in both the old and new tables. Any pointers or objects that are not at the same VMem address will cause Undefined Behaviour if accessed (unless the old page table is restored).
         * All kernel code you plan to call must be at the same addresses in both the old and new tables. Most important are INTERRUPT HANDLERS and the PANIC HANDLER (as well as common utilities such as klog). This also includes the activate() function and the function you called it from. (and the static variable that stores the page table)
         The easiest way to achieve the above three points is to map the kernel to the same position in every page table. This is why the kernel lives in the higher half - it should never be necessary to change its location in virtual memory.
         */
    pub unsafe fn activate(&self){
        let allocator = self.0._begin_active();
        
        // activate table
        let table_addr = ptaddr_virt_to_phys(allocator.get_page_table_ptr() as usize);
        klog!(Info, MEMORY_PAGING_CONTEXT, "Switching active context to 0x{:x}", table_addr);

        let ni = disable_interruptions();
        // Set active
        set_active_page_table(table_addr);  // TODO
        // store reference (and take old one)
        let oldpt = _ACTIVE_PAGE_TABLE.lock().replace(Self::clone_ref(&self));
        // Enable interruptions
        drop(ni);
        
        // Decrement reader count on old page table (if applicable)
        // Safety: Since the previous page table was activated using this function,
        //         which leaks a read guard, we can be sure that decrementing the
        //         counter here will be defined, working as if the guard had been dropped.
        // (N.B. we can't simply store the guard due to borrow checker limitations + programmer laziness)
        if let Some(old_table) = oldpt { unsafe {
            old_table.0._end_active();
        }}
    }
}
impl core::ops::Deref for PagingContext {
    type Target = LockedPageAllocator<BaseTLPageAllocator>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// = GUARDS =
#[derive(Debug,Clone,Copy)]
pub struct LPAWGOptions {
    pub(super) auto_flush_tlb: bool,
    pub(super) is_global_page: bool,

    pub(super) address_space_id: Option<AddressSpaceID>,
    pub(super) active_id: Option<ActivePageID>,
}
impl LPAWGOptions {
    pub(super) fn new_default() -> Self {
        Self {
            auto_flush_tlb: false,
            is_global_page: false,

            address_space_id: None,  // this should be filled in when the guard is taken
            active_id: None,  // this should be filled in when the guard is taken
        }
    }
}

pub struct LockedPageAllocatorWriteGuard<PFA: PageFrameAllocator, GuardT>
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA> 
  {
    guard: GuardT, allocator: LockedPageAllocator<PFA>,
    
    pub(super) options: LPAWGOptions,
}
//pub type TLPageAllocatorWriteGuard<'a> = LockedPageAllocatorWriteGuard<'a, BaseTLPageAllocator>;
pub type LPAWriteGuard<'a, PFA> = LockedPageAllocatorWriteGuard<PFA, HMutexGuard<'a, PFA>>;

macro_rules! ppa_define_foreach {
    // Note: body takes four variables from the outside: allocator, ptable, index, and offset (as well as any other args they specify).
    ($($fkw:ident)*: $fnname: ident, $pfaname:ident:&mut PFA, $allocname:ident:&PPA, $ptname:ident:&mut IPT, $idxname:ident:usize, $offname:ident:usize, $($argname:ident:$argtype:ty),*, $body:block, $sabody: block) => {
        $($fkw)* fn $fnname($pfaname: &mut impl PageFrameAllocator, $allocname: &PartialPageAllocation, parentoffset: usize, $($argname:$argtype),*){
            let $ptname = $pfaname.get_page_table_mut();
            // Call on current-level entries
            for item in $allocname.entries() {
                match item {
                    &PAllocItem::Page { index: $idxname, offset } => {
                        let $offname = parentoffset+offset;
                        $body
                    },
                    &PAllocItem::SubTable { index: $idxname, offset, .. } => {
                        let $offname = parentoffset+offset;
                        $sabody
                    },
                }
            }
            // Recurse into sub-allocators
            for item in $allocname.entries() {
                if let &PAllocItem::SubTable { index, offset, alloc: ref suballocation } = item {
                    let suballocator = $pfaname.get_suballocator_mut(index).expect("Allocation expected sub-allocator but none was found!");
                    Self::$fnname(suballocator,suballocation, parentoffset+offset, $($argname),*);
                }
            }
        }
    }
}
impl<PFA: PageFrameAllocator, GuardT> LockedPageAllocatorWriteGuard<PFA, GuardT> 
  where GuardT: core::ops::Deref<Target=PFA> + core::ops::DerefMut<Target=PFA>
  {
    fn meta(&self) -> &LPAMetadata {
        self.allocator.metadata()
    }
    fn get_page_table(&mut self) -> &mut PFA {
        &mut*self.guard
    }
    /* Used for logging */
    #[inline(always)]
    fn _pt_phys_addr(&mut self) -> usize {
        ptaddr_virt_to_phys(self.get_page_table().get_page_table_ptr() as usize)
    }
    #[inline(always)]
    fn _fmt_pa(&mut self, allocation: &PageAllocation<PFA>) -> alloc::string::String {
        alloc::format!("{:x}[{:?}]", self._pt_phys_addr(), allocation.allocation)
    }
    
    // Allocating
    // (we don't need to flush the TLB for allocation as the page has gone from NOT PRESENT -> NOT PRESENT - instead we flush it when it's mapped to an address)
    pub(super) fn allocate(&mut self, size: PageAllocationSizeT, alloc_strat: PageAllocationStrategies) -> Option<PageAllocation<PFA>> {
        let allocator = self.get_page_table();
        let allocation = allocator.allocate(size.get(), alloc_strat)?;
        Some(PageAllocation::new(LockedPageAllocator::clone_ref(&self.allocator), allocation))
    }
    pub(super) fn allocate_at(&mut self, addr: PageAlignedAddressT, size: PageAllocationSizeT) -> Option<PageAllocation<PFA>> {
        // Convert the address
        let addr: usize = addr.get();  // (page-alignment is really only an API restriction)
        // Subtract the offset from the vmem address (as we need it to be relative to the start of our table)
        let rel_addr = addr.checked_sub(self.meta().offset).unwrap_or_else(||panic!("Cannot allocate memory before the start of the page! addr=0x{:x} page_start=0x{:x}",addr,self.meta().offset));
        
        // Allocate
        let allocator = self.get_page_table();
        let allocation = allocator.allocate_at(rel_addr, size.get())?;
        let mut palloc = PageAllocation::new(LockedPageAllocator::clone_ref(&self.allocator), allocation);
        palloc.baseaddr_offset = 0; debug_assert!(addr == palloc.start().get());  // Rounding is no longer done implicitly, so the start() will be equal to the addr.
        Some(palloc)
    }
    /// Allocate page(s) dynamically such that the given physical address would be able to be mapped using a simple .set_addr (i.e. such that phys minus base would be page-aligned)
    pub(super) fn allocate_alignedoffset(&mut self, size: usize, alloc_strat: PageAllocationStrategies, phys_addr: usize) -> Option<PageAllocation<PFA>> {
        // Step 1: round down the physical address to page alignment
        use super::MIN_PAGE_SIZE;
        let (phys_addr_aligned, allocation_offset) = PageAlignedAddressT::new_rounded_with_excess(phys_addr);
        // Step 2: increase size by the remainder to compensate
        let allocated_size = size + allocation_offset;
        // Step 3: Allocate
        let mut allocation = self.allocate(PageAllocationSizeT::new_rounded(allocated_size), KALLOCATION_DYN_MMIO)?;
        allocation.baseaddr_offset += allocation_offset;
        Some(allocation)
    }
    
    /* Split the allocation into two separate allocations. The first one will contain bytes [0,n), and the second one will contain the rest.
       If necessary, huge pages will be split to meet the midpoint as accurately as possible.
       Note: It is not guaranteed that the second allocation will not be empty. */
    pub(super) fn split_allocation(&mut self, allocation: PageAllocation<PFA>, mid: PageAllocationSizeT) -> (PageAllocation<PFA>, PageAllocation<PFA>) {
        allocation.assert_pt_tag(self);
        let baseaddr_offset = allocation.baseaddr_offset;
        let (allocation, allocator, metadata) = allocation.leak();
        
        let (lhs, rhs) = Self::_split_alloc_inner(self.get_page_table(), allocation, mid.get());
        (
            PageAllocation { allocator: LockedPageAllocator::clone_ref(&allocator), allocation: lhs, metadata, baseaddr_offset },
            PageAllocation { allocator:                                 allocator , allocation: rhs, metadata, baseaddr_offset },
        )
    }
    fn _split_alloc_inner<SPF:PageFrameAllocator>(pfa: &mut SPF, allocation: PartialPageAllocation, mid: usize) -> (PartialPageAllocation, PartialPageAllocation) {
        use alloc::collections::vec_deque::VecDeque;
        let mut lhs = Vec::<PAllocItem>::new();
        let mut rhs = Vec::<PAllocItem>::new();
        
        let mut entries = VecDeque::from(allocation.into_entries());
        
        while let Some(item) = entries.pop_front() {
            if item.offset() < mid {
                // LHS or "pivot"
                if entries.front().map_or(true, |ni| ni.offset() > mid) {
                    // Next item is after the mid-point, so this item is the pivot (has the midpoint INSIDE it rather than on a boundrary)
                    // (if we're at the end and not past the midpoint, then the final item is considered the pivot)
                    match item {
                        PAllocItem::SubTable { index, offset, alloc: suballoc } => {
                            // It's a table, so we can split this if recurse
                            let suballocator = pfa.get_suballocator_mut(index).expect("Allocation expected sub-allocator but none was found!");
                            let lhs_size = mid.checked_sub(offset).unwrap();
                            let (left, right) = Self::_split_alloc_inner(suballocator, suballoc, lhs_size);
                            lhs.push(PAllocItem::SubTable { index, offset, alloc: left });
                            rhs.push(PAllocItem::SubTable { index, offset:offset+lhs_size, alloc: right });
                        }
                        
                        PAllocItem::Page { index, offset } => {
                            // Attempt to split the page
                            let result = pfa.split_page(index);
                            if let Ok(suballocation) = result {
                                // Success - split the newly created table
                                let suballocator = pfa.get_suballocator_mut(index).unwrap();
                                let lhs_size = mid.checked_sub(offset).unwrap();
                                let (left, right) = Self::_split_alloc_inner(suballocator, suballocation, lhs_size);
                                lhs.push(PAllocItem::SubTable { index, offset, alloc: left });
                                rhs.push(PAllocItem::SubTable { index, offset:offset+lhs_size, alloc: right });
                            } else {
                                // \_(o.o)_/
                                panic!("FUCK");
                            }
                        }
                    }
                } else {
                    // LHS
                    lhs.push(item);
                }
            } else {
                // RHS
                rhs.push(item);
            }
        }
        
        // Adjust RHS offsets so they are based on the RHS rather than the original allocation as a whole
        // (it's sorted (as this algo is stable and sorting is required) so we just take the first one lmao)
        if !rhs.is_empty() {
            let min_offset = rhs[0].offset();
            for item in &mut rhs {
                *item.offset_mut() -= min_offset;
            }
        }
        
        // Return allocations
        (
            PartialPageAllocation::new(lhs, SPF::PAGE_SIZE),
            PartialPageAllocation::new(rhs, SPF::PAGE_SIZE),
        )
    }
    
    /* drop() only gives us a reference so we have to make do */
    pub(self) fn dealloc(&mut self, allocation: &PageAllocation<PFA>){
        allocation.assert_pt_tag(self);
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Deallocating {}", self._fmt_pa(allocation));
        self.get_page_table().deallocate(&allocation.allocation);
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    
    // Managing allocations
    /* Set the base physical address for the given allocation. This also sets the PRESENT flag automatically. */
    pub(super) fn set_base_addr(&mut self, allocation: &PageAllocation<PFA>, base_addr: usize, mut flags: PageFlags){
        allocation.assert_pt_tag(self);
        
        if self.options.is_global_page { flags.mflags |= MappingSpecificPageFlags::GLOBAL; }
        
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Mapping {} to base addr {:x} (flags={:?})", self._fmt_pa(allocation), base_addr, flags);
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        unsafe {
            Self::_set_addr_inner(self.get_page_table(), allocation.into(), 0, base_addr, flags);
        }
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    ppa_define_foreach!(unsafe: _set_addr_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, base_addr: usize, flags: PageFlags, {
        ptable.set_huge_addr(index, base_addr+offset, flags);
    }, { ptable.add_subtable_flags::<false>(index, &flags); });
    
    /* Set the given allocation as absent (not in physical memory). */
    pub(super) fn set_absent(&mut self, allocation: &PageAllocation<PFA>, data: usize){
        // SAFETY: By holding a mutable borrow of ourselves (the allocator), we can verify that the page table is not in use elsewhere
        // (it is the programmer's responsibility to ensure the addresses are correct before they call unsafe fn activate() to activate it.
        allocation.assert_pt_tag(self);
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Mapping {} as absent (data={:x})", self._fmt_pa(allocation), data);
        unsafe {
            Self::_set_missing_inner(self.get_page_table(), allocation.into(), 0, data);
        }
        if self.options.auto_flush_tlb { self.invalidate_tlb(allocation) };
    }
    ppa_define_foreach!(unsafe: _set_missing_inner, allocator: &mut PFA, allocation: &PPA, ptable: &mut IPT, index: usize, offset: usize, data: usize, {
        ptable.set_absent(index, data);
    }, {});
    
    /* Invalidate the TLB entries for the given allocation (on the current CPU).
        Note: No check is performed to ensure that the allocation is correct nor that this page table is active, as the only consequence (provided all other code handling Page Tables / TLB is correct) is a performance hit from the unnecessary INVLPG operations + the resulting cache misses.
        Note: Using this method is unnecessary yourself. Usually it is provided by write_when_active or similar. */
    pub(super) fn invalidate_tlb(&mut self, allocation: &PageAllocation<PFA>){
        klog!(Debug, MEMORY_PAGING_CONTEXT, "Flushing TLB for {:?}", allocation.allocation);
        let vmem_offset = allocation.start();  // (vmem offset is now added by PageAllocation itself)
        // TODO let active_on = self.allocator.0.active_on.read(); // It's faster to hold the lock here than to clone as read/write contention on the same page allocator is negligble compared to contention on the heap allocator
        inval_tlb_pg(allocation.into(), allocation.metadata.offset, self.options.is_global_page, None);
    }
}

pub struct ForcedUpgradeGuard<'a, T> {
    guard: RwLockUpgradableGuard<'a, T>,
    ptr: &'a mut T,
}
impl<'a,T> ForcedUpgradeGuard<'a, T>{
    // SAFETY: HERE BE DRAGONS
    unsafe fn new(lock: &RwLock<T>, guard: RwLockUpgradableGuard<'a, T>) -> Self {
        use core::ops::Deref;
        // 👻👻👻👻👻
        // TODO: Find a better alternative
        // This dirty hack gives me the creeps
        let ptr = &mut *lock.data_ptr();
        Self {
            guard,
            ptr,
        }
    }
}
impl<T> core::ops::Deref for ForcedUpgradeGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.guard
    }
}
impl<T> core::ops::DerefMut for ForcedUpgradeGuard<'_, T>{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ptr
    }
}

// = ACTIVE OR SMTH? =
// the currently active page table on each CPU
use crate::sync::kspin::KMutex;
use crate::multitasking::{disable_interruptions, get_cpu_num};
static _ACTIVE_PAGE_TABLE: CpuLocal<KMutex<Option<PagingContext>>,false> = CpuLocal::new();

// = ALLOCATIONS =
// Note: Allocations must be allocated/deallocated manually
// and do not hold a reference to the allocator
// they are more akin to indices
pub struct PageAllocation<PFA: PageFrameAllocator> {
    allocator: LockedPageAllocator<PFA>,
    allocation: PartialPageAllocation,
    metadata: LPAMetadata,
    
    // baseaddr_offset is used in the event that an address passed to allocate_at is not page-aligned
    // the high-level API will handle the difference automatically
    baseaddr_offset: usize,
}
impl<PFA:PageFrameAllocator> PageAllocation<PFA> {
    pub(super) fn new(allocator: LockedPageAllocator<PFA>, allocation: PartialPageAllocation) -> Self {
        Self {
            metadata: *allocator.metadata(),
            allocator: allocator,
            allocation: allocation,
            baseaddr_offset: 0,
        }
    }
    fn assert_pt_tag(&self, allocator: &mut LockedPageAllocatorWriteGuard<PFA,impl core::ops::Deref<Target=PFA>+core::ops::DerefMut<Target=PFA>>){
        // TODO
    }
    
    /// Normalize this allocation, setting base to be equal to start
    /// This is necessary for managing allocations as part of a complex allocator,
    /// but may get in the way of attempts to offset-map addresses that aren't page-aligned
    pub fn normalize(&mut self) {
        self.baseaddr_offset = 0;
    }

    /* Deliberately leak the allocation, without freeing it.
       Use this instead of core::mem::forget, as this makes sure to drop the Arc<> which means the page table will be dropped when appropriate. */
    pub fn leak(self) -> (PartialPageAllocation, LockedPageAllocator<PFA>, LPAMetadata) {
        use core::{mem::MaybeUninit,ptr};
        let me = MaybeUninit::new(self);
        let me_ptr = me.as_ptr();
        
        // SAFETY: We must make sure to either extract or drop each field within ourselves
        //          as we will not be Drop'd
        unsafe{ (
            ptr::read(&(*me_ptr).allocation),
            ptr::read(&(*me_ptr).allocator ),
            ptr::read(&(*me_ptr).metadata  ),
        )}
    }
    
    /* Find the start address of this allocation in VMem */
    pub fn start(&self) -> PageAlignedAddressT {
        PageAlignedAddressT::new(canonical_addr(self.allocation.start_addr() + self.metadata.offset))
    }
    /* Find the "base" address of this allocation in VMem.
        This will be different from start() if e.g. the address passed to allocate_at was not page aligned.
        start() -> the very start of the allocation (including any padding added to ensure it is page-aligned).
        base() -> the VMem address that was requested. */
    pub fn base(&self) -> usize {
        self.start().get()+self.baseaddr_offset
    }
    /* Find the end address of this allocation in VMem. (exclusive) */
    pub fn end(&self) -> PageAlignedAddressT {
        PageAlignedAddressT::new(canonical_addr(self.allocation.end_addr() + self.metadata.offset))
    }
    /* The total size of this allocation from start to end (not adjusted by the "base" address) */
    pub fn size(&self) -> PageAllocationSizeT {
        PageAllocationSizeT::new_checked(self.allocation.size()).unwrap()
    }
    /* The length of this allocation following the base (i.e. from base to end)  */
    pub fn length_after_base(&self) -> usize {
        self.size().get() - self.baseaddr_offset
    }
    
    pub fn set_base_addr(&self, base_addr: usize, flags: PageFlags){
        self.allocator.write_when_active().set_base_addr(self, base_addr-self.baseaddr_offset, flags)
    }
    pub fn set_absent(&self, data: usize){
        self.allocator.write_when_active().set_absent(self, data)
    }
    pub fn flush_tlb(&self){
        self.allocator.write_when_active().invalidate_tlb(self)
    }
    
    pub fn split(self, mid: PageAllocationSizeT) -> (Self, Self) {
        let allocator = LockedPageAllocator::clone_ref(&self.allocator);
        let result = allocator.write_when_active().split_allocation(self, mid);
        result
    }
    pub fn alloc_downwards(&self, size: PageAllocationSizeT) -> Option<Self> {
        let start = PageAlignedAddressT::new(self.start().get()-size.get());
        self.allocator.allocate_at(start, size)
    }
}
impl<PFA:PageFrameAllocator> core::ops::Drop for PageAllocation<PFA> {
    fn drop(&mut self){
        self.allocator.write_when_active().dealloc(self);
    }
}
impl<'a,PFA:PageFrameAllocator> From<&'a PageAllocation<PFA>> for &'a PartialPageAllocation {
    fn from(value: &'a PageAllocation<PFA>) -> Self {
        &value.allocation
    }
}

impl<PFA:PageFrameAllocator> core::fmt::Debug for PageAllocation<PFA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageAllocation").field("alloc", &self.allocation).finish()
    }
}

use alloc::boxed::Box;
use core::hint::spin_loop;
use core::ops::Deref;
use crate::memory::paging::tlb::{ActivePageID, AddressSpaceID};
/* Any page allocation, regardless of PFA. */
pub trait AnyPageAllocation: core::fmt::Debug + Send {
    fn normalize(&mut self);

    fn start(&self) -> PageAlignedAddressT;
    fn end(&self) -> PageAlignedAddressT;
    fn size(&self) -> PageAllocationSizeT;

    /// Get the physical address of the page table
    /// Intended for use as a heuristic only
    fn pt_phys_addr(&self) -> usize;

    fn set_base_addr(&self, base_addr: usize, flags: PageFlags);
    fn set_absent(&self, data: usize);
    fn flush_tlb(&self);

    /// Split this page allocation in half
    fn split_dyn(self: Box<Self>, mid: PageAllocationSizeT) -> (Box<dyn AnyPageAllocation>,Box<dyn AnyPageAllocation>);
    /// Allocate more virtual memory directly below this allocation
    /// This is made available as a new allocation
    fn alloc_downwards_dyn(&self, size: PageAllocationSizeT) -> Option<Box<dyn AnyPageAllocation>>;
}
impl<PFA:PageFrameAllocator + Send + Sync + 'static> AnyPageAllocation for PageAllocation<PFA> {
    fn normalize(&mut self){ self.normalize() }

    fn start(&self) -> PageAlignedAddressT { self.start() }
    fn end(&self) -> PageAlignedAddressT { self.end() }
    fn size(&self) -> PageAllocationSizeT { self.size() }
    fn pt_phys_addr(&self) -> usize { self.allocator.get_phys_addr() }

    fn set_base_addr(&self, base_addr: usize, flags: PageFlags) { self.set_base_addr(base_addr, flags) }
    fn set_absent(&self, data: usize) { self.set_absent(data) }
    fn flush_tlb(&self) { self.flush_tlb() }

    // Note: Self is not always Sized
    fn split_dyn(self: Box<Self>, mid: PageAllocationSizeT) -> (Box<dyn AnyPageAllocation>,Box<dyn AnyPageAllocation>) {
        let (left, right) = self.split(mid);
        (Box::new(left) as Box<dyn AnyPageAllocation>,
         Box::new(right) as Box<dyn AnyPageAllocation>)
    }
    fn alloc_downwards_dyn(&self, size: PageAllocationSizeT) -> Option<Box<dyn AnyPageAllocation>> {
        self.alloc_downwards(size).map(|pa|Box::new(pa) as Box<dyn AnyPageAllocation>)
    }
}

pub type TopLevelPageAllocation = PageAllocation<BaseTLPageAllocator>;
pub type TLPageFrameAllocator = BaseTLPageAllocator;