
// TODO: find a faster/better way to do this?
// NOTE: This requires the kernel heap to be initialised!

use alloc::boxed::Box;
use crate::multitasking::fixedcpulocal::FixedCpuLocals;

#[inline(always)]
pub fn _set_fixed_cpu_locals(cpulocals: FixedCpuLocals){
    // Leak the CPU local and get the address
    let cpu_local_ptr = (Box::leak(Box::new(cpulocals)) as *mut FixedCpuLocals) as usize;
    // Create a pointer to the pointer (yep)
    let cpu_local_ptr_ptr = (Box::leak(Box::new(cpu_local_ptr)) as *mut usize) as usize;
    // Point GS:0 -> CPU local ptr
    unsafe{
        core::arch::asm!(
            "mov edx,0",
            "mov ecx,0xC0000101",
            "wrmsr",
            in("eax") cpu_local_ptr_ptr, out("edx") _, out("ecx") _
        );
    }
}
#[inline(always)]
pub fn _load_fixed_cpu_locals() -> &'static FixedCpuLocals {
    // GS:0 = cpu_local_ptr
    // FIXME: GSbase is only 32-bits, so we need to offset the returned value by HIGHER_HALF_OFFSET (if that even still exists somewhere in the code)
    let cpu_locals_ptr: usize;
    unsafe{
        core::arch::asm!(
            "mov {x},gs:0",
            x = out(reg) cpu_locals_ptr,
        );
    }
    unsafe { &*(cpu_locals_ptr as *const FixedCpuLocals) }
}