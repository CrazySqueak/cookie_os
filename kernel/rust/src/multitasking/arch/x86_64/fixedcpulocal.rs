
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
    
    // Stage 2.1: Pre-allocate heap memory
    // Since moving the cpulocals into the Box will invalidate the copy GS is currently pointed at,
    //  we must pre-allocate the heap memory before moving them.
    let cpu_local_heap = Box::<FixedCpuLocals>::new_uninit();
    let cpu_local_ptr_heap = Box::<usize>::new_uninit();
    // Now that the heap memory is allocated, we can discard the items on the stack
    // This discard is placed here to ensure that they live long enough
    let _ = (_s1_cpu_local_ptr, _s1_cpu_local_ptr_ptr);
    
    // Stage 2.2: Move cpu locals into heap memory
    // Leak the CPU local and get the address
    let cpu_local_ptr = (Box::leak(Box::write(cpu_local_heap, cpulocals)) as *mut FixedCpuLocals) as usize;
    // Create a pointer to the pointer (yep)
    let cpu_local_ptr_ptr = (Box::leak(Box::write(cpu_local_ptr_heap, cpu_local_ptr)) as *mut usize) as usize;
    // Point GS:0 -> CPU local ptr
    _point_gs(cpu_local_ptr_ptr);
}
#[inline(always)]
pub fn _load_fixed_cpu_locals() -> &'static FixedCpuLocals {
    // GS:0 = cpu_local_ptr
    // GSbase is only 32-bits, so we need to offset the returned value by HIGHER_HALF_OFFSET
    // Since FixedCpuLocals are allocated on the kernel heap, we can use KERNEL_PTABLE_VADDR as the offset
    const HIGHER_HALF_OFFSET: usize = crate::memory::paging::global_pages::KERNEL_PTABLE_VADDR;
    const FCL_PTR_PTR_ADDR: usize = 0 + HIGHER_HALF_OFFSET;  // we load GS:FCL_PTR_PTR_ADDR
    
    let cpu_locals_ptr: usize;
    unsafe{
        core::arch::asm!(
            "mov {x},gs:[{addr_reg}]",
            x = lateout(reg) cpu_locals_ptr,
            addr_reg = in(reg) FCL_PTR_PTR_ADDR,  // 64-bit immediates and x86_64 mix like oil and water, so we have to put it in a register first (just like when first CALLing into the higher half)
        );
    };
    unsafe { &*(cpu_locals_ptr as *const FixedCpuLocals) }
}