
/// Declare the "api" for system calls, including the tag numbers.
///
/// This does not declare the actual ABI used, but instead provides a high-level overview
/// of the system calls, separate from their (platform-specific) implementation.
macro_rules! declare_syscall_tag {
    {
        tag = $tagvis:vis enum($tagty:ty) $tagname:ident;
        num_syscalls = $nsvis:vis const $nsname:ident;
        iface_def_macro = $rmvis:vis macro $rmname:ident;

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

        // A macro to allow inspection of the interface
        macro_rules! $rmname {
            (@build_handler_table, $htvis:vis $htname:ident, $htmod:ident) => {
                #[repr(C)]
                #[allow(non_snake_case)]
                $htvis struct $htname {
                    $($callname: $htmod::$callname),+
                }
            };
            $((@get_tag $callname -> const $cname:ident) => {
                const $cname: $tagty = $calltag;
            };)+
        }
        $rmvis use $rmname;

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
pub(crate) use declare_syscall_tag;

#[cfg(feature = "examples")]
declare_syscall_tag! {
    tag = pub enum(u32) ExampleSyscall;
    num_syscalls = pub const NUM_EXAMPLE_SYSCALLS;
    iface_def_macro = pub(crate) macro example_iface;

    /// Test0
    extern syscall(0x00) fn Test0(x, y);
    /// Test1
    extern syscall(0x01) fn Test1() -> abc;
    /// Test2
    extern syscall(0x02) fn Test2((x,y), z) -> x_or_y;
}

/// Declare the ABI for system calls, including creating a "handler table" to hold them.
macro_rules! declare_syscall_abi {
    {
        $(#[doc=$ccdoc:literal])*
        callconv = $callconv:literal;
        iface_def = $ifdmacro:path;

        syscall_fn_types = $htmvis:vis mod $htmname:ident;
        $(#[doc=$htdoc:literal])*
        handler_table = $htvis:vis struct $htname:ident;
        $(#[doc=$indoc:literal])*
        invokers = $invis:vis mod $inname:ident;

        abi {
            $(
                $(#[doc=$fn_abi_doc:literal])*
                syscall fn $callname:ident($($regtype:ty),*) $(-> $rty:ty)?;
            )+
        }

        invoke($itname:ident) $invoker:block
    } => {
        $htmvis mod $htmname {
            $(
                $(#[doc=$fn_abi_doc])*
                pub type $callname = extern $callconv fn($($regtype),*) $(-> $rty)?;
            )+
        }

        // TODO $(#[doc=$htdoc])*
        #[cfg(feature="handle")]
        $ifdmacro!(@build_handler_table, $htvis $htname, $htmname);

        #[cfg(feature="invokers")]
        $invis mod $inname {
            $(
                $(#[doc=$fn_abi_doc])*
                #[allow(non_snake_case)]
                #[naked]
                pub extern $callconv fn $callname($(_:$regtype),*) $(-> $rty)? {
                    $ifdmacro!(@get_tag $callname -> const $itname);
                    $invoker
                }
            )+
        }
    };
}
pub(crate) use declare_syscall_abi;

#[cfg(all(feature = "examples", target_arch = "x86_64"))]
declare_syscall_abi! {
    /// Arguments are passed in: rdi, rsi, rdx, rcx, r8, and r9
    /// Additional scratch registers: rax (tag / return value), r10, r11
    callconv = "sysv64";
    iface_def = example_iface;

    syscall_fn_types = pub mod example_handler_types;
    handler_table = pub struct ExampleHandlerTable;
    invokers = pub mod example_invokers;

    abi {
        syscall fn Test0(u64, u64);
        syscall fn Test1();
        /// Test2 - X and Y are packed into a single u64.
        syscall fn Test2(u64, u8) -> u32;
    }
    invoke(TAG){unsafe{
        // Load the tag value into eax, then perform a syscall
        // Since the invoker function is naked, we can patch this in
        // maybe?
        core::arch::asm!("mov {tag}, eax", "syscall", tag = const TAG, options(noreturn))
    }}
}

