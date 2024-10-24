use crate::syscore::marshal::FFIMarshalled;

macro_rules! define_syscalls {
    (@resolve_rtype, $rtype:ty) => {$rtype};
    (@resolve_rtype) => {()};
    
    {
        tag = $tagvis:vis enum($tagty:ty) $tagname:ident;
        handler_table = $hvis:vis struct $hname:ident;
        handler_types = $hmvis:vis mod $hmname:ident;
        invokers = $ivis:vis mod $iname:ident;
        num_syscalls = $nsvis:vis const $nsname:ident;
        //use $abip:path as abi;
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
        
        // Syscall handler table
        #[cfg(feature="handle")]
        #[allow(non_snake_case)]
        #[repr(C)]
        $hvis struct $hname {
            $(
                $(#[doc=$doc])*
                $callname: Option<$hmname::$callname>,
            )+
        }
        // Syscall function pointers
        #[cfg(feature="handle")]
        $hmvis mod $hmname {
            use super::*;
            $(
                $(#[doc=$doc])*
                pub type $callname = extern "sysv64" fn($($argtype),*) $(-> $rtype)?;
            )+
        }

        // Syscall invoker utilites
        #[cfg(feature="invokers")]
        $ivis mod $iname {
            use super::*;
            //use $abip as __abi;
            $(
                $(#[doc=$doc])*
                #[allow(non_snake_case)]
                pub fn $callname($($argname:$argtype),*) $(-> $rtype)? {
                    todo!()
                }
            )+
            pub mod automarshall {
                use super::*;
                $(
                    $(#[doc=$doc])*
                    #[allow(non_snake_case)]
                    #[allow(non_camel_case_types)]
                    pub fn $callname
                    <$($argname),*>($($argname:$argname),*) $(-> $rtype)?
                    where $($argname: ::core::convert::Into<$argtype>),*
                    {
                        $(let $argname: $argtype = $argname.into();)*
                        super::$callname($($argname),*)
                    }
                )+
            }
        }
        
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
pub(crate) use define_syscalls;

#[cfg(feature="examples")]
define_syscalls! {
    tag = pub enum(u32) SyscallTag;
    handler_table = pub struct SyscallHandlerTable;
    handler_types = pub mod handlers;
    invokers = pub mod invokers;
    num_syscalls = pub const NUM_SYSCALLS;
    
    extern syscall(0x00) fn Test0(x:u32, y:u64);
    extern syscall(0x01) fn Test1() -> FFIMarshalled<bool>;
    extern syscall(0x02) fn HaltCatchFireF00F() -> FFIMarshalled<()>;
    extern syscall(0x03) fn BleegleTheBlarp(z:u32,a:u32) -> u64;
    extern syscall(4) fn DoesLiterallyNothing();
    extern syscall(5) fn Test47(a:u32,b:FFIMarshalled<bool>) -> FFIMarshalled<core::ptr::NonNull<u32>>;
}

#[cfg(feature="examples")]
pub fn x() -> u32 {
    NUM_SYSCALLS
}

#[cfg(feature="examples")]
pub fn y() {
    invokers::automarshall::Test47(69u32, true);
}