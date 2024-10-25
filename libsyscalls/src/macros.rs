
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

        // Handler table
        // N.B. table[tag] should = the handler
        #[repr(C)]
        #[allow(non_snake_case)]
        $htvis struct $htname<RegisterSet,ErrorCode,HandlerType>
          where HandlerType: Fn(RegisterSet)->Result<RegisterSet,ErrorCode>
        {
            $(
                $(#[doc=$doc])*
                $callname: Option<HandlerType>,
            )+
            __phantom: ::core::marker::PhantomData<(RegisterSet,ErrorCode)>,
        }
        impl<RegisterSet,ErrorCode,HandlerType> $htname<RegisterSet,ErrorCode,HandlerType>
          where HandlerType: Fn(RegisterSet)->Result<RegisterSet,ErrorCode> {
            const _CHECK_1:() = {assert!(size_of::<Option<HandlerType>>() == size_of::<Option<fn()>>())};
            const _CHECK_2:() = {assert!(size_of::<Self>() == size_of::<[Option<fn()>;$nsname as usize]>())};
            // const _CHECK_3:() = unsafe{assert!(core::mem::transmute::<usize,Option<HandlerType>>(0).is_none())};
        }
        impl<RegisterSet,ErrorCode,HandlerType> core::ops::Index<$tagname> for $htname<RegisterSet,ErrorCode,HandlerType>
          where HandlerType: Fn(RegisterSet)->Result<RegisterSet,ErrorCode> {
            type Output = Option<HandlerType>;
            fn index(&self, index: $tagname) -> &Self::Output {
                // Safety: We are repr(C), the same size as a [Option<fn()>;num_syscalls],
                // and each handler pointer is the same size as a Option<fn()>
                // Thus, our layout is guaranteed
                unsafe {
                    let self_arr: &[Option<fn()>;$nsname as usize] = core::mem::transmute(self);
                    let index: $tagty = index.into();
                    let index = index as usize;
                    let handler_raw = &self_arr[index];
                    let handler: &Option<HandlerType> = core::mem::transmute(handler_raw);
                    handler
                }
            }
        }
        impl<RegisterSet,ErrorCode,HandlerType> core::ops::IndexMut<$tagname> for $htname<RegisterSet,ErrorCode,HandlerType>
          where HandlerType: Fn(RegisterSet)->Result<RegisterSet,ErrorCode> {
            fn index_mut(&mut self, index: $tagname) -> &mut Self::Output {
                unsafe {
                    let self_arr: &mut [Option<fn()>;$nsname as usize] = core::mem::transmute(self);
                    let index: $tagty = index.into();
                    let index = index as usize;
                    let handler_raw = &mut self_arr[index];
                    let handler: &mut Option<HandlerType> = core::mem::transmute(handler_raw);
                    handler
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

