
// NOTE: This requires the kernel heap to be initialised!

use alloc::boxed::Box;
use crate::multitasking::fixedcpulocal::FixedCpuLocals;

#[inline]
fn _point_gs(cpu_local_ptr_ptr: usize){
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

#[inline]
pub fn _set_fixed_cpu_locals(cpulocals: FixedCpuLocals){
    // Note: The heap allocator requires FixedCpuLocals to be initialised, so we set GSBase twice
    // First, we point it to our cpulocals on the stack, so we can use the allocator
    // Second, we allocate the locals+ptr on the heap, and point it to them.
    
    // Stage 1
    // Get the address of the locals on the stack
    let _s1_cpu_local_ptr = core::ptr::addr_of!(cpulocals) as usize;
    // Pointer to the pointer
    let _s1_cpu_local_ptr_ptr = core::ptr::addr_of!(_s1_cpu_local_ptr) as usize;
    // Point GS to the pointer
    _point_gs(_s1_cpu_local_ptr_ptr);
    
    // Stage 2
    // Leak the CPU local and get the address
    let cpu_local_ptr = (Box::leak(Box::new(cpulocals)) as *mut FixedCpuLocals) as usize;
    // Create a pointer to the pointer (yep)
    let cpu_local_ptr_ptr = (Box::leak(Box::new(cpu_local_ptr)) as *mut usize) as usize;
    // Point GS:0 -> CPU local ptr
    _point_gs(cpu_local_ptr_ptr);
    
    // Now that GS is pointed at heap memory, we can discard the items on the stack
    // This discard is placed here to ensure that they live long enough
    let _ = (_s1_cpu_local_ptr, _s1_cpu_local_ptr_ptr);
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