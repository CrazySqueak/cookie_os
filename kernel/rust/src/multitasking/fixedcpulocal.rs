//! Fixed CPU-local values.
//! Unlike regular CpuLocals, these cannot be allocated dynamically, but must be statically put here.
//! This makes it useful for values such as the cpu number, and so on.
use super::arch::fixedcpulocal as arch;

pub struct FixedCpuLocals {
    pub cpu_id: usize,
    pub current_ni_guard: super::interruptions::FCLCurrentNIGuard,
}
/* Call once per CPU, early on. */
pub fn init_fixed_cpu_locals(){
    let cpu_id = NEXT_CPU_ID.fetch_add(1, Ordering::Acquire);
    
    // Store
    arch::_set_fixed_cpu_locals(FixedCpuLocals {
        cpu_id: cpu_id,
        current_ni_guard: super::interruptions::FCLCurrentNIGuardDefault,
    });
}
#[inline(always)]
pub fn get_fixed_cpu_locals() -> &'static FixedCpuLocals {
    arch::_load_fixed_cpu_locals()
}

// CPU ID - each cpu is assigned an OS-derived "CPU ID" for easy sorting and identification and stuff
use core::sync::atomic::{AtomicUsize,Ordering};
static NEXT_CPU_ID: AtomicUsize = AtomicUsize::new(0);

/* Get the CPU number for the local CPU.
    CPU numbers are assigned sequentially, so CPU 0 is the bootstrap processor, CPU 1 is the first AP to start, etc. */
#[inline(always)]
pub fn get_cpu_num() -> usize {
    get_fixed_cpu_locals().cpu_id
}