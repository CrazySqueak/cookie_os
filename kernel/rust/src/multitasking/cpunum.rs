// CPU ID - each cpu is assigned an OS-derived "CPU ID" for easy sorting and identification and stuff
use core::sync::atomic::{AtomicU16,Ordering};
use super::arch::cpunum as arch;
static NEXT_CPU_ID: AtomicU16 = AtomicU16::new(0);
/* Call once per CPU, early on. */
pub fn init_cpu_num(){
    let cpu_id = NEXT_CPU_ID.fetch_add(1, Ordering::Acquire);
    // Store
    arch::_store_cpu_num(cpu_id);
}
/* Get the CPU number for the local CPU.
    CPU numbers are assigned sequentially, so CPU 0 is the bootstrap processor, CPU 1 is the first AP to start, etc. */
#[inline(always)]
pub fn get_cpu_num() -> usize {
    arch::_load_cpu_num().into()
}