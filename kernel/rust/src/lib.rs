#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![feature(box_into_inner)]

// i'm  exhausted by these warnings jeez
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

extern crate alloc;

pub mod memory;
pub mod multitasking;

// arch-specific code lives in "x::arch" for some modules
macro_rules! arch_specific_module {
    ($v:vis mod $name:ident) => {
        $v mod $name { cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                mod x86_64;
                pub use x86_64::*;
            } else {
                compile_error!(concat!("This architecture is unsupported as it does not have an implementation for the '",stringify!($name),"' module!"));
            }
        }}
    }
}
pub(crate) use arch_specific_module;

#[no_mangle]
pub extern "sysv64" fn _kstart() -> ! {
    todo!()
}
#[no_mangle]
pub extern "sysv64" fn _kstart_ap() -> ! {
    todo!()
}

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
// This variable tracks if we are already aborting due to a panic
// If this is true when a panic occurs, the panic handler simply halts (detecting that an infinite panic loop has occurred)
static _ABORTING: AtomicBool = AtomicBool::new(false);
// To avoid contention issues but keep the utility, only the panicking CPU will print the "end of panic" messages
static _PANICKING_CPU: AtomicUsize = AtomicUsize::new(0xFF69420101);  // whatever value I put here as the default won't be read anyway, so might as well make it something significant
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    todo!()
}