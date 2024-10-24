pub struct RegisterSet {
    /// In: Tag of the syscall to execute.
    /// Out: Error code on failure, or zero on success.
    pub eax: u32,

    // In: First four parameters
    // Out: Inline return value (up to 256bits)
    //      (rdi = first 8 bytes, rsi = second 8 bytes, rdx = third 8 bytes, rcx = final 8 bytes)
    pub rdi: u64, pub rsi: u64,
    pub rdx: u64, pub rcx: u64,
    /// Extra parameters are put into a struct, with the struct pointer placed in r8.
    pub r8: *mut u8,
    /// Return value pointer, required if the return value is >256bits in size.
    pub r9: *mut u8,
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

// N.B. The callee will have to manually unpack the registers into a single RegisterSet
//      in the interrupt/STAR handler using raw ASM, as rust cannot guarantee that it will not
//      overwrite any registers before that point.

// TODO: Packing/unpacking impls
