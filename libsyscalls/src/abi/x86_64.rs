use core::mem::MaybeUninit;
use crate::syscore::marshal::FFIMarshalled;
use crate::syscore::safety::SyscallFFISafe;

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
    pub r8: *const u8,
    /// Return value pointer, required if the return value is >256bits in size.
    pub r9: *mut u8,
}

// Moving to/from registers
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

// Packing/unpacking into a RegisterSet
// This is OK because all SyscallFFISafe types are repr(C) and thus have a defined size
pub unsafe fn pack_return_value<R:SyscallFFISafe>(reg: &mut RegisterSet, val: R) {
    reg.eax = 0;  // success

    // Begin packing
    if size_of::<R>() > 4*size_of::<u64>() {
        let ptr = reg.r9 as *mut R;
        core::ptr::write(ptr, val);
    } else {
        let mut bytes: MaybeUninit<[u8; 4*size_of::<u64>()]> = MaybeUninit::uninit();
        core::ptr::write(bytes.as_mut_ptr() as *mut R, val);
        let values: [u64; 4] = core::mem::transmute(bytes.assume_init());
        reg.rdi = values[0]; reg.rsi = values[1];
        reg.rdx = values[2]; reg.rcx = values[3];
    }
}
pub unsafe fn unpack_return_value<R:SyscallFFISafe>(reg: RegisterSet) -> Result<R,u32> {
    if reg.eax != 0 { return Err(reg.eax); }

    // Begin unpacking
    if size_of::<R>() > 4*size_of::<u64>() {
        // Passed by pointer
        let ptr = reg.r9 as *mut R;
        let read = core::ptr::read(ptr); drop((reg,ptr));  // (we drop the RegisterSet here to ensure the pointer doesn't get re-used)
        Ok(read)
    } else {
        // Passed in registers
        let values: [u64; 4] = [reg.rdi,reg.rsi,reg.rdx,reg.rcx];
        let bytes: [u8; 4*size_of::<u64>()] = core::mem::transmute(values);
        let bytes_ptr = core::ptr::addr_of!(bytes);
        let rval = core::ptr::read(bytes_ptr as *const R); drop((reg,values,bytes,bytes_ptr));
        Ok(rval)
    }
}
pub unsafe fn unpack_zst_return_value(reg: RegisterSet) -> Result<(),u32> {
    if reg.eax != 0 { Err(reg.eax) }
    else { Ok(()) }
}

macro_rules! pack_argument_values {
    (@pack_main_reg, $regs:ident.$regn:ident <- $valn:ident: $valt:ty ) => {
        const _:() = { $crate::syscore::safety::assert_ffi_safe::<$valt>() };
        if size_of::<$valt>() > size_of::<u64>() {
            // Passed as pointer
            let val_ptr = ::core::ptr::addr_of!($valn);
            $regs.$regn = val_ptr as u64;
        } else {
            // Passed in register
            let reg_ptr = ::core::ptr::addr_of_mut!($regs.$regn);
            core::ptr::write(reg_ptr as *mut $valt, $valn);
        }
    };


    (
        $($val1n:ident: $val1t:ty)?
        , $($val2n:ident: $val2t:ty)?
        , $($val3n:ident: $val3t:ty)?
        , $($val4n:ident: $val4t:ty)?
    ) => {{
        let mut regs = $crate::abi::RegisterSet {
            eax: 0, rdi: 0, rsi: 0, rdx: 0, rcx: 0, r8: ::core::ptr::null_mut(), r9: ::core::ptr::null_mut()
        };

        $($crate::abi::pack_argument_values!(@pack_main_reg, regs.rdi <- $val1n: $val1t);)?
        $($crate::abi::pack_argument_values!(@pack_main_reg, regs.rdi <- $val2n: $val2t);)?
        $($crate::abi::pack_argument_values!(@pack_main_reg, regs.rdi <- $val3n: $val3t);)?
        $($crate::abi::pack_argument_values!(@pack_main_reg, regs.rdi <- $val4n: $val4t);)?

        regs
    }};
    (
        $($val1n:ident: $val1t:ty)?
        , $($val2n:ident: $val2t:ty)?
        , $($val3n:ident: $val3t:ty)?
        , $($val4n:ident: $val4t:ty)?
        $(, $valXn:ident: $valXt:ty)+
    ) => {{
        let mut regs = $crate::abi::pack_argument_values!($($val1n: $val1t)?,$($val2n: $val2t)?,$($val3n: $val3t)?,$($val4n: $val4t)?);

        $crate::syscore::marshal::ffi_struct! {
            pub(crate) extern struct ExtraArgs {
                $($valXn: $valXt),+
            }
        }
        let extras = ExtraArgs { $($valXn),+ };
        let extras_ptr = core::ptr::addr_of!(extras);
        regs.r8 = extras_ptr as *const u8;
        regs
    }};

    () => {$crate::abi::pack_argument_values!(,,,)};
    ($val1n:ident:$val1t:ty) => {$crate::abi::pack_argument_values!($val1n:$val1t,,,)};
    ($val1n:ident:$val1t:ty, $val2n:ident:$val2t:ty) => {$crate::abi::pack_argument_values!($val1n:$val1t,$val2n:$val2t,,)};
    ($val1n:ident:$val1t:ty, $val2n:ident:$val2t:ty, $val3n:ident:$val3t:ty) => {$crate::abi::pack_argument_values!($val1n:$val1t,$val2n:$val2t,$val3n:$val3t,)};
}
pub(crate) use pack_argument_values;

macro_rules! unpack_argument_values {
    (@unpack_main_reg, $regs:ident.$regn:ident -> $valn:ident: $valt:ty ) => {
        const _:() = { $crate::syscore::safety::assert_ffi_safe::<$valt>() };
        if size_of::<$valt>() > size_of::<u64>() {
            // Passed as pointer
            let val_ptr = $regs.$regn as *const $valt;
            // TODO: validate pointer
            $valn = core::ptr::read(val_ptr);
        } else {
            // Passed in register
            let reg_ptr = ::core::ptr::addr_of!($regs.$regn);
            $valn = core::ptr::read(reg_ptr as *const $valt);
        }
    };


    ($regs:ident ->
        $($val1n:ident: $val1t:ty)?
        , $($val2n:ident: $val2t:ty)?
        , $($val3n:ident: $val3t:ty)?
        , $($val4n:ident: $val4t:ty)?
    ) => {{
        $($crate::abi::unpack_argument_values!(@unpack_main_reg, $regs.rdi -> $val1n: $val1t);)?
        $($crate::abi::unpack_argument_values!(@unpack_main_reg, $regs.rdi -> $val2n: $val2t);)?
        $($crate::abi::unpack_argument_values!(@unpack_main_reg, $regs.rdi -> $val3n: $val3t);)?
        $($crate::abi::unpack_argument_values!(@unpack_main_reg, $regs.rdi -> $val4n: $val4t);)?
    }};
    ($regs:ident ->
        $($val1n:ident: $val1t:ty)?
        , $($val2n:ident: $val2t:ty)?
        , $($val3n:ident: $val3t:ty)?
        , $($val4n:ident: $val4t:ty)?
        $(, $valXn:ident: $valXt:ty)+
    ) => {{
        $crate::abi::unpack_argument_values!($regs -> $($val1n: $val1t)?,$($val2n: $val2t)?,$($val3n: $val3t)?,$($val4n: $val4t)?);

        $crate::syscore::marshal::ffi_struct! {
            pub(crate) extern struct ExtraArgs {
                $($valXn: $valXt),+
            }
        }
        let extras_ptr = $regs.r8 as *const ExtraArgs;
        // TODO: Validate pointer
        let extras = core::ptr::read(extras_ptr);
        ExtraArgs { $($valXn),+ } = extras;
    }};

    () => {$crate::abi::unpack_argument_values!($regs -> ,,,)};
    ($regs:ident -> $val1n:ident:$val1t:ty) => {$crate::abi::unpack_argument_values!($regs -> $val1n:$val1t,,,)};
    ($regs:ident -> $val1n:ident:$val1t:ty, $val2n:ident:$val2t:ty) => {$crate::abi::unpack_argument_values!($regs -> $val1n:$val1t,$val2n:$val2t,,)};
    ($regs:ident -> $val1n:ident:$val1t:ty, $val2n:ident:$val2t:ty, $val3n:ident:$val3t:ty) => {$crate::abi::unpack_argument_values!($regs -> $val1n:$val1t,$val2n:$val2t,$val3n:$val3t,)};
}
pub(crate) use unpack_argument_values;

macro_rules! pack_tag_and_alloc_rval {
    ($regs:ident <- $tag:expr) => {
        $regs.eax = $tag;
    };
    ($regs:ident <- $tag:expr, $rval:ident: $rtype:ty) => {
        $regs.eax = $tag;
        let mut $rval: ::core::mem::MaybeUninit<$rtype> = ::core::mem::MaybeUninit::uninit();
        $regs.r9 = $rval.as_mut_ptr() as *mut u8;
    };
}
pub(crate) use pack_tag_and_alloc_rval;

unsafe fn test1() {
    let x: FFIMarshalled<bool> = true.into(); let y: u128 = 2; let z: u32 = 3; let zz: u64 = 4;
    let a: u32 = 5; let b: u32 = 6;
    let regs = pack_argument_values!(
        x: FFIMarshalled<bool>, y: u128, z: u32, zz: u64, a: u32, b: u32
    );
    let (x,y,z,zz,a,b);
    unpack_argument_values!(regs -> x: FFIMarshalled<bool>, y: u128, z: u32, zz: u64, a: u32, b: u32);
}