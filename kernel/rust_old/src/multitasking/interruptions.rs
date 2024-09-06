use crate::sync::{cpulocal::CpuLocal,kspin::KRwLockRaw};
use core::sync::atomic::{AtomicBool,Ordering};

/// Perform the inner actions without interruptions
/// All maskable interrupts will be delayed
/// Any attempt to yield to the scheduler is considered to be a bug and will cause a panic
pub fn without_interruptions<R>(closure: impl FnOnce()->R) -> R {
    crate::lowlevel::_without_interrupts(||{
        let old = INTERRUPTIONS_DISABLED.get().swap(true,Ordering::Acquire);
        let result = closure();
        INTERRUPTIONS_DISABLED.get().store(old,Ordering::Release);
        result
    })
}
static INTERRUPTIONS_DISABLED: CpuLocal<AtomicBool,KRwLockRaw> = CpuLocal::new();
pub fn are_interruptions_disabled() -> bool {
    crate::lowlevel::_without_interrupts(||INTERRUPTIONS_DISABLED.get().load(Ordering::Relaxed))
}
