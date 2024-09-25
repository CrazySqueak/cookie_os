mod featureflags;
mod gdt;

pub fn init_bsp() {
    // Init MSR
    featureflags::init_msr();
    // Init GDT
    gdt::init();
}
pub fn init_bsp_2() {
}

pub fn init_ap() {
    // Init MSR
    featureflags::init_msr_ap();
    // Init GDT
    gdt::init();
}
pub fn init_ap_2() {
}
// TODO