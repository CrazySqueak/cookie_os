use core::sync::atomic::{AtomicBool,Ordering};

// == CPU NUMBER ASSIGNMENT ==
// CPU ID - each cpu is assigned an OS-derived "CPU ID" for easy sorting and identification and stuff
use core::sync::atomic::AtomicU16;
static NEXT_CPU_ID: AtomicU16 = AtomicU16::new(0);
pub type CpuID = u16;
/* Call once per CPU, early on. */
pub fn init_cpu_num(){
    let cpu_id = NEXT_CPU_ID.fetch_add(1, Ordering::Acquire);
    // Store
    todo!()//crate::lowlevel::_store_cpu_num(cpu_id);
}
/* Get the CPU number for the local CPU.
    CPU numbers are assigned sequentially, so CPU 0 is the bootstrap processor, CPU 1 is the first AP to start, etc. */
#[inline(always)]
pub fn get_cpu_num() -> CpuID {
    todo!()//crate::lowlevel::_load_cpu_num().into()
}

// == BOOTSTRAP SCHEDULER READY? ==
pub static BSP_SCHEDULER_READY: AtomicBool = AtomicBool::new(false);
#[inline]
pub fn is_bsp_scheduler_initialised() -> bool {
    BSP_SCHEDULER_READY.load(Ordering::Relaxed)
}