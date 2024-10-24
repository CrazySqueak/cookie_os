pub struct RegisterSet {
    /// In: Tag of the syscall to execute.
    /// Out: Error code on failure, or zero on success.
    eax: u32,

    // In: First four parameters
    // Out: Inline return value (up to 256bits)
    //      (rdi = first 8 bytes, rsi = second 8 bytes, rdx = third 8 bytes, rcx = final 8 bytes)
    rdi: u64, rsi: u64,
    rdx: u64, rcx: u64,
    /// Extra parameters are put into a struct, with the struct pointer placed in r8.
    r8: *mut (),
    /// Return value pointer, required if the return value is >256bits in size.
    r9: *mut (),
}

pub unsafe fn invoke(reg: &mut RegisterSet){
    core::arch::asm!("syscall",
        // Tag/error code
        inout("eax") reg.eax,

        // Arguments/return value
        inout("rdi") reg.rdi, inout("rsi") reg.rsi,
        inout("rdx") reg.rdx, inout("rcx") reg.rcx,
        // Special arguments
        in("r8") reg.r8, in("r9") reg.r9,

        // The other two scratch registers are clobbered
        out("r10") _, out("r11") _,
    )
}
