
mod gdt;
mod interrupts;
mod lowlevel;
pub (in crate) mod multiboot;  // TEMP: TODO ADD PUBLIC API

pub use lowlevel::{init, halt, without_interrupts};