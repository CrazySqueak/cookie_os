
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
mod smp;
mod featureflags; use featureflags::init_msr; use featureflags::init_msr_ap;

pub use lowlevel::{halt, without_interrupts};
pub use smp::start_all_processors_xapic_acpi as start_all_processors;

use crate::coredrivers::system_apic;

/* Early BSP initialisation prior to paging/extendedheap/etc. setup. */
pub fn init1_bsp() {
    init_msr();
}
/* Late BSP initialisation to be done after paging / memory is initialised. */
pub fn init2_bsp() {
    // xAPIC
    system_apic::init_local_apic();
    
    // GDT + Interrupts
    gdt::init();
    interrupts::init();
}
/* Early AP initialisation */
pub fn init1_ap() {
    init_msr_ap();
}
/* Late AP initialisation */
pub fn init2_ap() {
    // xAPIC
    system_apic::init_local_apic();
    
    gdt::init();
    interrupts::init_ap();
}


// TODO: Put somewhere
// TODO: find a faster/better way to do this?
#[inline(always)]
pub fn _store_cpu_num(id: u16){
    unsafe{
        core::arch::asm!(
            "mov edx,0",
            "mov ecx,0xC0000101",
            "wrmsr",
            in("eax") id, out("edx") _, out("ecx") _
        );
    }
}
#[inline(always)]
pub fn _load_cpu_num() -> u16{
    let id: u16;
    unsafe{
        core::arch::asm!(
            "mov edx,0",
            "mov ecx,0xC0000101",
            "rdmsr",
            out("eax") id, out("edx") _, out("ecx") _
        );
    }
    id
}