
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;

pub use lowlevel::{HIGHER_HALF_OFFSET, halt, without_interrupts};

pub fn init() {
    gdt::init();
    interrupts::init();
}