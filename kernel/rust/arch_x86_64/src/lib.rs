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
pub mod gdt;
pub mod interrupts;
pub mod controlflow;
pub mod multiboot;
pub mod context_switch;
pub mod smp;
pub mod featureflags;

pub use controlflow::{init1_bsp,init2_bsp,init1_ap,init2_ap};
pub use controlflow::{halt,without_interrupts};