//! Kernel Spin
//! 
//! K-locks are primitive spinlocks that loop indefinitely until the required condition is met.
//! It is recommended that you DO NOT YIELD the current task while you hold a K-lock, as doing so may deadlock the kernel.
//!
//! To ensure that this is taken into account, ALL uses of K-locks (both KMutex and KRwLock as well as their raw variants) must be listed here:
//! MODULE      FILE            ITEM                NOTES
//! coredrivers uart_x86_64.rs  Serial1             The serial port is stored in a KMutex. The lock is held indefinitely by kernel_log, for logging.
//! memory      kernel_heap.rs  Global Allocator    The global heap allocator is stored in a KMutex, for obvious reasons. This is safely abstracted away by the Allocator API.
//! paging      paging_api.rs   ACTIVE_PAGE_TABLE   A reference to the active page table is stored in a K-locked CpuLocal. This is safely abstracted away by the activate() method.
//! scheduler   scheduler.rs    Scheduler State     Kept in a KLocked CpuLocal for obvious reasons. Safely abstracted away by yield_to_scheduler(command) and the other pub methods.
//! multitaskng interruptions.. Interrupt Disabled? Interrupts are cleared before the variable is updated. Safely abstracted away.
//! sync        cpulocal.rs     CpuLocalNoInterrupt Abstracted away inside the CpuLocalNoInterrupts
//! lowlevel    featureflags.rs MSR_FEATURE_FLAGS   Used before interrupts/multitaskng are configured
//! lowlevel    interrupts.rs   GLOBAL_IDT          Used before interrupts/multitaskng are configured
//! lowlevel    gdt.rs          GLOBAL_GDT          Used before interrupts/multitaskng are configured

use super::spin::*;
use core::hint::spin_loop;

pub struct CpuSpin;
impl Relax for CpuSpin {
    fn relax(){
        spin_loop()
    }
}

pub type KMutexRaw = SpinMutexRaw<CpuSpin>;
pub type KMutex<T> = SpinMutex<T,CpuSpin>;
pub type KMutexGuard<'a,T> = SpinMutexGuard<'a,T,CpuSpin>;
pub type MappedKMutexGuard<'a,T> = MappedSpinMutexGuard<'a,T,CpuSpin>;
pub type KRwLockRaw = SpinRwLockRaw<CpuSpin>;
pub type KRwLock<T> = SpinRwLock<T,CpuSpin>;
pub type KRwLockReadGuard<'a,T> = SpinRwLockReadGuard<'a,T,CpuSpin>;
pub type KRwLockWriteGuard<'a,T> = SpinRwLockWriteGuard<'a,T,CpuSpin>;
pub type MappedKRwLockReadGuard<'a,T> = MappedSpinRwLockReadGuard<'a,T,CpuSpin>;
pub type MappedKRwLockWriteGuard<'a,T> = MappedSpinRwLockWriteGuard<'a,T,CpuSpin>;