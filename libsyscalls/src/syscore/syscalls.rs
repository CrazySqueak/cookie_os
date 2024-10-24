use crate::syscore::marshal::FFIMarshalled;

/// Define the syscall interface.
///
/// The syscall interface defines the names and tag numbers of the syscalls,
/// as well as their "theoretical" argument and return types.
macro_rules! define_syscall_interface {
    (@resolve_rtype, $rtype:ty) => {$rtype};
    (@resolve_rtype) => {()};
    
    {
        tag = $tagvis:vis enum($tagty:ty) $tagname:ident;
        interfacedefs = $(#[$scdmeta:meta])* $scdvis:vis macro $scdname:ident;
        num_syscalls = $nsvis:vis const $nsname:ident;
        $(
            $(#[doc=$doc:literal])*
            extern syscall($callid:literal) fn $callname:ident ($($argname:ident: $argtype:ty),*) $(-> $rtype:ty)?;
        )+
    } => {
        // Syscall ID enum
        $crate::syscore::marshal::ffi_enum! {
            #[warn(non_camel_case_types, reason="Syscalls should have upper camel case names")]
            $tagvis extern($tagty) enum $tagname {
                $(
                    $(#[doc=$doc])*
                    $callname = $callid,
                )+
            }
        }

        $(#[$scdmeta:meta])*
        macro_rules! $scdname {
            ()=>{} // TODO
        }
        $scdvis use $scdname;
        
        // Assert that all syscall numbers are continuous and in order. (this is necessary to allow the handler table to be used as a lookup table using the syscall ID as an index)
        // Fun side-effect: The value of i after this has run is the total number of syscalls! Might as well use it (was probably going to need it eventually).
        $nsvis const $nsname: $tagty = {
            let mut i: $tagty = 0;
            $(
                assert!(($tagname::$callname as $tagty) == i, concat!("\nSystem call IDs must be numbered consecutively, starting from 0! (if needed, add Reserved69 for ID 0x69, ReservedF0 for ID 0xF0, etc.)\nEncountered at ",stringify!($callname), " with ID ", stringify!($callid), "\n"));
                i += 1;
            )+
            i
        };
        // Assert that all parameter types are safe
        $(
            $(const _:() = $crate::syscore::safety::assert_ffi_safe::<$argtype>();)*
            $(const _:() = $crate::syscore::safety::assert_ffi_safe::<$rtype>();)?
        )+
    }
}
pub(crate) use define_syscall_interface;

/// Define the syscall ABI.
macro_rules! define_syscall_abi {
    {
        for interface $iface:path;
        as $mvis:vis mod $mname:ident;
        in {
            tag -> $tagreg:ident;
            // $($(#[$mreg_meta:meta])* $mreg_name:ident: $mreg_type:ty,)+
            // $(,)?
        }
        out{
            errcode -> $errreg:ident;
            // $($(#[$rreg_meta:meta])* $rreg_name:ident: $rreg_type:ty,)*
            // $(,)?
        }

        $($callname:ident {
            args = ( $($caname:ident: $catype:ty),* ) -> ( $($crgname:ident: $crgtype:ty),* );

            args->reg $a2rg_body:block
            args<-reg $rg2a_body:block

            return = ($($rvname:ident: $rvtype:ty)?) -> ( $($rrgname:ident: $rrgtype:ty),* );
            return->reg $r2rg_body:block
            return<-reg $rg2r_body:block
        })+
    } => {
        $mvis mod $mname {
            #[allow(unused_imports)]
            use super::*;

            $(#[allow(non_snake_case)] pub mod $callname {
                #[allow(unused_imports)]
                use super::*;

                #[cfg(feature="invokers")]
                pub fn invoke($($caname:$catype),*) -> Result<($($rvtype)?),u32> {
                    // args -> registers
                    $(let $crgname: $crgtype;)*
                    $a2rg_body

                    // TODO: pack registers
                    todo!(); // TODO: invoke
                    let ($($rrgname),*): ($($rrgtype),*) = todo!();  // TODO: unpack registers

                    // registers -> return values
                    $(let $rvname: $rvtype;)?
                    $rg2r_body
                    Ok(($($rvname)?))
                }
                #[cfg(feature="handle")]
                pub fn handle_invoke($($crgname:$crgtype),*) -> Result<($($rrgtype),*),u32> {
                    // registers -> args
                    $(let $caname: $catype;)*
                    $rg2a_body

                    // TODO: call handler
                    let ($($rvname)?): ($($rvtype)?) = todo!();

                    // return values -> registers
                    $(let $rrgname: $rrgtype;)*
                    $r2rg_body
                    Ok(($($rrgname),*))
                }
                // pub fn reg2args($($crgname:$crgtype),*) -> ($($catype),*) {
                //     $(let $caname: $catype;)*
                //     $rg2a_body
                //     ($($caname),*)
                // }
            })+
        }
    };
}
pub(crate) use define_syscall_abi;

#[cfg(feature="examples")]
define_syscall_interface! {
    tag = pub enum(u32) SyscallTag;
    interfacedefs = pub(crate) macro example_sc_defs;
    num_syscalls = pub const NUM_SYSCALLS;
    
    extern syscall(0x00) fn Test0(x:u32, y:u64);
    extern syscall(0x01) fn Test1() -> FFIMarshalled<bool>;
    extern syscall(0x02) fn HaltCatchFireF00F() -> FFIMarshalled<()>;
    extern syscall(0x03) fn BleegleTheBlarp(z:u32,a:u32) -> u64;
    extern syscall(4) fn DoesLiterallyNothing();
    extern syscall(5) fn Test47(a:u32,b:FFIMarshalled<bool>) -> FFIMarshalled<core::ptr::NonNull<u32>>;
}
#[cfg(all(feature="examples",target_arch="x86_64"))]
define_syscall_abi! {
    for interface example_sc_defs;
    as pub mod example_x86_64;
    in {
        tag -> eax;
        // // Main registers
        // rdi: u64, rsi: u64, rdx: u64, rcx: u64,
        // /// The "extra parameters" register.
        // r8: u64,
    }
    out {
        errcode -> eax;
        // // Main return registers
        // rdi: u64, rsi: u64, rdx: u64, rcx: u64,
    }

    Test0 {
        args = (x: u32, y: u64) -> (edi: u32, rsi: u64);
        args->reg {
            edi = x; rsi = y;
        }
        args<-reg {
            x = edi; y = rsi;
        }

        return = () -> ();
        return->reg{}
        return<-reg{}
    }
    BleegleTheBlarp {
        args = (z: u32, a: u32) -> (edi: u32, esi: u32);
        args->reg {
            edi = z; esi = a;
        }
        args<-reg {
            z = edi; a = esi;
        }

        return = (rval: u64) -> (rdi: u64);
        return->reg{
            rdi = rval
        }
        return<-reg{
            rval = rdi
        }
    }
}

#[cfg(feature="examples")]
pub fn x() -> u32 {
    NUM_SYSCALLS
}

//#[cfg(feature="examples")]
//pub fn y() {
//    invokers::automarshall::Test47(69u32, true);
//}