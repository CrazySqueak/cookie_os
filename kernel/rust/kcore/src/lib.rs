#![no_std]

// i'm  exhausted by these warnings jeez
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

extern crate alloc;

pub mod logging;
//pub use logging::{klog,emergency_kernel_log};
pub mod util;
pub mod sync;