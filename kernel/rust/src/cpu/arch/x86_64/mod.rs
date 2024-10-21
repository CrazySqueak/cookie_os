mod featureflags;
mod gdt;
mod interrupts;
mod undefined_interrupt_handler_impl;

pub fn init_bsp() {
    // Init MSR
    featureflags::init_msr();
    // Init GDT
    gdt::init();
    // Init interrupts
    interrupts::init();
}
pub fn init_bsp_2() {
}

pub fn init_ap() {
    // Init MSR
    featureflags::init_msr_ap();
    // Init GDT
    gdt::init();
    // Init interrupts
    interrupts::init_ap();
}
pub fn init_ap_2() {
}
// TODO