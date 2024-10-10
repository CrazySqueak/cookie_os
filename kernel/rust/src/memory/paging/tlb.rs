use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::num::NonZeroU16;
use core::ops::{AddAssign, Deref};
use core::sync::atomic::{AtomicU16, Ordering};
use lazy_static::lazy_static;
use crate::memory::paging::api::{ClASIDs, PPAandOffset, PPAandOffsetOwned};
use crate::memory::paging::sealed::PartialPageAllocation;
use crate::multitasking::cpulocal::CpuLocal;
use crate::sync::kspin::{KMutex, KMutexGuard, MappedKMutexGuard};

/** Represents an Address space ID (`ASID`/`PCID` on x86 systems)

If [Unassigned](AddressSpaceID::Unassigned), then the respective page doesn't belong to any address space (or it's unsupported on this CPU),
 so the TLB entries for it should be flushed on every context switch (unless it's global).
If [Assigned](AddressSpaceID::Assigned), then the respective page belongs to a specific address space, so its TLB entries should only be flushed
 if it is specifically signified.
*/
#[derive(Debug, Clone,Copy, Eq,PartialEq)]
pub enum AddressSpaceID {
    /// No associated ASID - equivalent to "0" on x86 systems
    /// This ASID may be shared between multiple processes, but must also be flushed when switching between them.
    Unassigned,

    #[cfg(feature="__IallowNonZeroASID")]
    Assigned(NonZeroU16),
}
impl AddressSpaceID {
    pub fn into_u16(self) -> u16 {
        match self {
            Self::Unassigned => 0,
            #[cfg(feature="__IallowNonZeroASID")]
            Self::Assigned(x) => x.get(),
        }
    }
}
impl Default for AddressSpaceID {
    // TODO: Consider the ramifications of using Unassigned to mean both "unavailable" and "unset", and whether they are beneficial or a burden
    fn default() -> Self {
        Self::Unassigned
    }
}
// Thanks to rustc magic, this takes up zero bytes if __IallowNonZeroASID is disabled.

/** Represents an "Active Page ID"

This is assigned to a page when it is activated, and unassigned once it is no longer active.
It is intended for use on multi-CPU systems when performing a shootdown,
 as a "destination select" operand wherever possible,
 to avoid interrupting CPUs that don't have the page active.

N.B. CPUs that don't have a given page active will check for its ASID before switching to it,
 and flush entries then, so you only need to fire an interrupt if the page is *currently* active on that CPU.

Note: For global pages, this shouldn't be present, as global pages are assumed to be present on all CPUs.
*/
#[derive(Debug, Clone,Copy, Eq,PartialEq)]
pub struct ActivePageID(NonZeroU16);

static NEXT_CREATED_ACTIVE_ID: AtomicU16 = AtomicU16::new(1);
lazy_static! {
    static ref FREE_ACTIVE_IDS: KMutex<Vec<ActivePageID>> = {
        const N_AT_START: usize = 64;
        let mut v = Vec::with_capacity(N_AT_START);
        for i in 0..N_AT_START {
            let id = NEXT_CREATED_ACTIVE_ID.fetch_add(1,Ordering::Relaxed);
            v.push(ActivePageID(NonZeroU16::new(id).unwrap()))
        }
        v.reverse();
        KMutex::new(v)
    };
}
/// Google RAII guard
/// (holy hell)
///
/// Use [::get()](OwnedActiveID::get) to get the held value.
pub struct OwnedActiveID(ActivePageID);
impl OwnedActiveID {
    pub fn get(&self) -> ActivePageID {
        self.0
    }
}
impl Into<ActivePageID> for OwnedActiveID {
    fn into(self) -> ActivePageID {
        self.get()
    }
}
impl Into<ActivePageID> for &OwnedActiveID {
    fn into(self) -> ActivePageID {
        self.get()
    }
}
impl Drop for OwnedActiveID {
    fn drop(&mut self) {
        FREE_ACTIVE_IDS.lock().push(self.get())
    }
}

/// Take an active ID from the free list, to mark a now-active paging context
pub fn next_free_active_id() -> OwnedActiveID {
    let mut v = FREE_ACTIVE_IDS.lock();
    if v.is_empty() {
        let id = NEXT_CREATED_ACTIVE_ID.fetch_add(1,Ordering::Relaxed);
        if id == 0 { drop(v); return next_free_active_id(); }  // if id == 0, then silently wrap above it instead of panicking
        OwnedActiveID(ActivePageID(NonZeroU16::new(id).unwrap()))
    } else {
        OwnedActiveID(v.pop().unwrap())
    }
}

#[derive(Debug)]
pub struct PendingTLBFlush {
    /// Targeted non-global page allocations to be flushed
    pub target_nonglobal_allocations: Vec<PPAandOffsetOwned>,
    /// Whether the full address space should be flushed (excluding global pages)
    pub full_addr_space: bool,
}
impl Default for PendingTLBFlush {
    fn default() -> Self {
        Self {
            target_nonglobal_allocations: Vec::new(),
            full_addr_space: false,
        }
    }
}
#[derive(Debug,Default)]
pub struct PendingGlobalFlushes {
    pub by_alloc: Vec<PPAandOffsetOwned>
}
#[derive(Debug,Default)]
struct PendingFlushes {
    /// Allocations to be flushed for the "global address space" (for all ASIDs / even if the global flag is set)
    global: PendingGlobalFlushes,
    /// Allocations to be flushed per-ASID
    per_asid: Vec<PendingTLBFlush>,
}
static PENDING_FLUSHES: CpuLocal<KMutex<PendingFlushes>,true> = CpuLocal::new();

/// Get the pending flushes for the given address space (on the current CPU)
pub fn get_pending_flushes_for_asid(asid: AddressSpaceID) -> MappedKMutexGuard<'static, PendingTLBFlush> {
    let mut mg = PENDING_FLUSHES.lock();
    let pf = &mut mg.per_asid;
    if pf.len() <= asid.into_u16() as usize {
        let n_missing = asid.into_u16() as usize - pf.len() + 1;
        pf.reserve(n_missing);
        for i in 0..n_missing { pf.push(Default::default()) }
    }
    KMutexGuard::map(mg, |mg|&mut mg.per_asid[asid.into_u16() as usize])
}
/// Get the pending flushes for the global address space (on the current CPU)
pub fn get_pending_flushes_for_global() -> MappedKMutexGuard<'static, PendingGlobalFlushes> {
    let mg = PENDING_FLUSHES.lock();
    KMutexGuard::map(mg, |mg|&mut mg.global)
}
/// Push some flushes onto some CPUs' queues (per ASID).\
/// Acts based on whether the ASID for each CPU is assigned:
///  - For ASID [Unassigned](AddressSpaceID::Unassigned) - calls `push_fn` if the CPU has page #`active_id` currently active.
///  - For ASID [Assigned](AddressSpaceID::Assigned) - always calls `push_fn`.
/// N.B. This only occurs for CPUs who are stored in the CpuLocal (so only CPUs that have activated this page in the past).
pub fn push_flushes(active_id: ActivePageID, asids: &ClASIDs, push_fn: impl Fn(MappedKMutexGuard<'static, PendingTLBFlush>)) {
    for (cpu_id, asid) in CpuLocal::get_all_cpus(asids) {
        let asid = asid.lock();
        let should_push = match asid.asid {
            AddressSpaceID::Unassigned => {
                // Unassigned is always flushed between switches, so we only need to flush if it's currently active
                let is_active = true; let _ = active_id; // TODO
                is_active
            },
            #[cfg(feature="__IallowNonZeroASID")]
            AddressSpaceID::Assigned(x) => {
                true  // we could still be cached here, even if we're inactive, so we'd better push ourselves to make sure
            },
        };
        if should_push {
            // Call push_fn to push
            let mg = CpuLocal::get_for(&PENDING_FLUSHES, cpu_id).lock();
            if mg.per_asid.len() <= asid.asid.into_u16() as usize {
                // Not present
                // [get_pending_flushes_for_asid] is called in two scenarios:
                //  1. Before the given page is activated, to check what flushes need to be done (and perform them)
                //  2. When the TLB_SHOOTDOWN interrupt is triggered (it's sent to all CPUs with an active_id matching the one being flushed)
                // And get_pending_flushes_for_asid ensures that the Vec holds a pending flush object, for holding any pending flushes
                // Therefore, if the vec doesn't contain a flush object for this CPU/ASID, we know that the page cannot have been cached on this CPU as the ASID was never activated.
                // This is less of a killer optimization and more of a "no point handling this in any other way" situation.
                continue;
            }
            push_fn(KMutexGuard::map(mg, |pf|&mut pf.per_asid[asid.asid.into_u16() as usize]));
        }
    }
}
/// Push some flushes onto all CPUs' queues, for the global address space rather than any specific ASID
pub fn push_global_flushes(push_fn: impl Fn(MappedKMutexGuard<'static, PendingGlobalFlushes>)) {
    for (cpu_id, pgf) in CpuLocal::get_all_cpus(&PENDING_FLUSHES) {
        let mg = pgf.lock();
        push_fn(KMutexGuard::map(mg,|pf|&mut pf.global))
    }
}


// ACTUAL FLUSH LOGIC
use crate::memory::paging::arch::{inval_local_tlb_pg, inval_tlb_pg_broadcast, inval_tlb_pg_broadcast_global};
use crate::memory::paging::PageAllocation;

/// Flush the given non-global allocation for the given active_id/asids.
pub fn flush_local(active_id: ActivePageID, asids: &ClASIDs, allocation: PPAandOffset) {
    if inval_tlb_pg_broadcast(active_id, allocation, asids) {
        // Nothing to do, we've broadcast it
    } else {
        // We must use our workaround
        // Push flushes to target CPUs
        push_flushes(active_id, asids, |mut flush_info|{
            flush_info.target_nonglobal_allocations.push(allocation.to_owned())
        });
        // TODO: Send IPI to handle added flushes
    }
}
/// Flush the given global allocation.
pub fn flush_global(allocation: PPAandOffset) {
    if inval_tlb_pg_broadcast_global(allocation) {
        // Nothing to do, we've broadcast it
    } else {
        // Push flushes to all CPUs
        push_global_flushes(|mut flush_info|{
            flush_info.by_alloc.push(allocation.to_owned())
        });
        // TODO: Send IPI
    }
}
/// Flush the entire (non-global) address space for the given ASID on the given CPU, if it is not currently active (determined using active_id)
///
/// Returns true if successful, false if it failed because the page was in use.
pub fn flush_asid(cpu_id: usize, active_id: ActivePageID, asid: AddressSpaceID) -> bool {
    if let Some(state) = CpuLocal::get_for(&PENDING_FLUSHES, cpu_id).lock().per_asid.get_mut(asid.into_u16() as usize) {
        let is_active: bool = false; let _ = active_id; // TODO
        if is_active { return false; }  // fail if the page is active
        state.full_addr_space = true;
        // We don't need to send an IPI because the page is not currently in use - it will be flushed when next switched to

        true
    } else { true }  // Not present?? either way this means it's inactive
}