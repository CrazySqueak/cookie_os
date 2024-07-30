
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
pub mod context_switch;  // testing

pub use lowlevel::{HIGHER_HALF_OFFSET, halt, without_interrupts};

pub fn init() {
    lowlevel::init_msr();
    gdt::init();
    interrupts::init();
}