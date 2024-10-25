
/// Declare the "api" for system calls, including the tag numbers.
///
/// This does not declare the actual ABI used, but instead provides a high-level overview
/// of the system calls, separate from their (platform-specific) implementation.
macro_rules! declare_syscalls {
    {
        tag = $tagvis:vis enum($tagty:ty) $tagname:ident;
        error_code = $eevis:vis enum($eety:ty) $eename:ident;
        num_syscalls = $nsvis:vis const $nsname:ident;
        handler_table = $htvis:vis struct $htname:ident;

        $(
            $(#[doc=$doc:literal])*
            extern syscall($calltag:literal) fn $callname:ident ($($argdocname:ty),*) $( -> $docrt:ty)?;
        )+

        $(  ;
            $(#[doc=$errdoc:literal])*
            error code $errname:ident = $errcode:literal;
        )*
    } => {
        // Syscall ID enum
        #[derive(Debug,Clone,Copy)]
        $tagvis enum $tagname {
            $(
                $(#[doc=$doc])*
                #[warn(non_camel_case_types, reason="Syscalls should have upper camel case names")]
                $callname = $calltag,
            )+
        }
        impl ::core::convert::From<$tagname> for $tagty {
            fn from(value: $tagname) -> Self {
                match value {
                    $($tagname::$callname => $calltag),+
                }
            }
        }
        impl ::core::convert::TryFrom<$tagty> for $tagname {
            type Error = (u32, u32);
            fn try_from(value: $tagty) -> Result<Self, Self::Error> {
                match value {
                    $($calltag => Ok(Self::$callname),)+
                    _ => Err((value, $nsname)),
                }
            }
        }
        // Error Code enum
        #[repr($eety)]
        #[derive(Debug,Clone,Copy)]
        $eevis enum $eename {
            /// Success signifies that no error is present.
            /// In an ideal world, this would be an Option<$eename> instead,
            /// but the motherfucking orphan rule prevents me from implementing TryFrom and From
            /// so instead we get this.
            Success = 0 as $eety,
            /// Unsupported signals that the system call is unsupported, has no defined handler, is undefined, or is otherwise unrecognised.
            UnsupportedCall = u128::MAX as $eety,  // truncate MAX down to the correct value
            $(
                $(#[doc=$errdoc])*
                #[warn(non_camel_case_types, reason="Syscall error codes should have upper camel case names")]
                $errname = $errcode,
            )*
        }
        impl ::core::convert::From<$eename> for $eety {
            fn from(value: $eename) -> Self {
                match value {
                    $eename::Success => 0,
                    $eename::UnsupportedCall => u128::MAX as $eety,
                    $($eename::$errname => $errcode,)*
                }
            }
        }
        impl ::core::convert::TryFrom<$eety> for $eename {
            type Error = u32;
            fn try_from(value: $eety) -> Result<Self, Self::Error> {
                if value == u128::MAX as $eety { return Ok($eename::UnsupportedCall); }
                match value {
                    0 => Ok($eename::Success),
                    $($errcode => Ok($eename::$errname),)*
                    _ => Err(value),
                }
            }
        }

        /// Handler table
        /// This is dispatched from a rust stub which handles the Result<> and register interactions,
        ///  therefore we don't need any fancy "extern" or unsafe layout-manip business.
        /// If a given handler is `None`, then ErrUnsupported should be returned to the calling application.
        #[allow(non_snake_case)]
        #[derive(Default)]
        #[cfg(feature = "handle")]
        $htvis struct $htname<RegisterSet> {
            $(
                $(#[doc=$doc])*
                $callname: Option<fn(&mut RegisterSet)->Result<(),$eename>>,
            )+
        }
        impl<RegisterSet> $htname<RegisterSet> {
            pub fn dispatch(&self, tag: $tagname, rs: &mut RegisterSet) -> Result<(),$eename> {
                self[tag].ok_or($eename::UnsupportedCall)?(rs)
            }
        }
        impl<RegisterSet> core::ops::Index<$tagname> for $htname<RegisterSet> {
            type Output = Option<fn(&mut RegisterSet)->Result<(),$eename>>;
            fn index(&self, index: $tagname) -> &Self::Output {
                match index {
                    $($tagname::$callname => &self.$callname,)+
                }
            }
        }
        impl<RegisterSet> core::ops::IndexMut<$tagname> for $htname<RegisterSet> {
            fn index_mut(&mut self, index: $tagname) -> &mut Self::Output {
                match index {
                    $($tagname::$callname => &mut self.$callname,)+
                }
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
    };
}
pub(crate) use declare_syscalls;

#[cfg(feature = "examples")]
declare_syscalls! {
    tag = pub enum(u32) ExampleSyscall;
    error_code = pub enum(u32) ExampleSyscallErrorCode;
    num_syscalls = pub const NUM_EXAMPLE_SYSCALLS;
    handler_table = pub struct ExampleHandlerTable;

    /// Test0
    extern syscall(0x00) fn Test0(x, y);
    /// Test1
    extern syscall(0x01) fn Test1() -> abc;
    /// Test2
    extern syscall(0x02) fn Test2((x,y), z) -> x_or_y;
}

fn x(ht: ExampleHandlerTable<()>){
    let x = ht[ExampleSyscall::Test0];
    todo!()
}