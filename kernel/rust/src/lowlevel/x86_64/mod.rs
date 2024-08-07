
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
pub mod context_switch;  // testing
pub mod smp;
mod apic;

pub use lowlevel::{halt, without_interrupts};
pub use smp::get_cpu_id;

/* Early initialisation prior to paging/extendedheap/etc. setup. */
pub fn init() {
    lowlevel::init_msr();
    gdt::init();
    interrupts::init();
}
/* Later initialisation after paging/etc. (e.g. multicore support) */
pub fn init2(){
    apic::map_local_apic_mmio();
    unsafe{ apic::enable_apic(); }
    smp::init_multiprocessing();
}