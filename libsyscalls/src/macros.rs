
/// Declare the "api" for system calls, including the tag numbers.
///
/// This does not declare the actual ABI used, but instead provides a high-level overview
/// of the system calls, separate from their (platform-specific) implementation.
macro_rules! declare_syscalls {
    {
        tag = $tagvis:vis enum($tagty:ty) $tagname:ident;
        num_syscalls = $nsvis:vis const $nsname:ident;
        handler_table = $htvis:vis struct $htname:ident;

        $(
            $(#[doc=$doc:literal])*
            extern syscall($calltag:literal) fn $callname:ident ($($argdocname:ty),*) $( -> $docrt:ty)?;
        )+
    } => {
        // Syscall ID enum
        #[repr($tagty)]
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

        /// Handler table
        /// This is dispatched from a rust stub which handles the Result<> and register interactions,
        ///  therefore we don't need any fancy "extern" or unsafe layout-manip business.
        /// If a given handler is `None`, then ErrUnsupported should be returned to the calling application.
        #[allow(non_snake_case)]
        #[derive(Default)]
        #[cfg(feature = "handle")]
        $htvis struct $htname<RegisterSet,ErrorCode> {
            $(
                $(#[doc=$doc])*
                $callname: Option<fn(RegisterSet)->Result<RegisterSet,ErrorCode>>,
            )+
        }
        impl<RegisterSet,ErrorCode> core::ops::Index<$tagname> for $htname<RegisterSet,ErrorCode> {
            type Output = Option<fn(RegisterSet)->Result<RegisterSet,ErrorCode>>;
            fn index(&self, index: $tagname) -> &Self::Output {
                match index {
                    $($tagname::$callname => &self.$callname,)+
                }
            }
        }
        impl<RegisterSet,ErrorCode> core::ops::IndexMut<$tagname> for $htname<RegisterSet,ErrorCode> {
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
    num_syscalls = pub const NUM_EXAMPLE_SYSCALLS;
    handler_table = pub struct ExampleHandlerTable;

    /// Test0
    extern syscall(0x00) fn Test0(x, y);
    /// Test1
    extern syscall(0x01) fn Test1() -> abc;
    /// Test2
    extern syscall(0x02) fn Test2((x,y), z) -> x_or_y;
}

fn x(ht: ExampleHandlerTable<(),u32>){
    let x = ht[ExampleSyscall::Test0];
    todo!()
}