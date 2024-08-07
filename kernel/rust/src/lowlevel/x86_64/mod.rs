
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
pub mod context_switch;  // testing
pub mod smp;

pub use lowlevel::{halt, without_interrupts};
pub use smp::get_cpu_id;

pub fn init() {
    lowlevel::init_msr();
    gdt::init();
    interrupts::init();
}