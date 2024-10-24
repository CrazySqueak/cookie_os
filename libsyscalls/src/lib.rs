#![no_std]

mod syscore;
pub mod syscalls;

#[cfg_attr(target_arch = "x86_64",path="abi/x86_64/mod.rs")]
mod abi;