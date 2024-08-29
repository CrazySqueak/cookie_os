#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![feature(box_into_inner)]

// i'm  exhausted by these warnings jeez
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

#[macro_export]
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

extern crate alloc;
pub mod logging;
pub mod util;
pub mod sync;
pub mod forward;

//arch_specific_module!(pub mod lowlevel);
