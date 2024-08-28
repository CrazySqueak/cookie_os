use core::arch::asm;

pub fn halt() -> ! {
    // SAFETY: This code does not modify memory besides disabling interupts, and does not return
    // it is an end point after which nothing more should happen
    // or something
    // i really shouldn't be allowed to use unsafe{} when I'm this tired lmao
    unsafe {
        asm!("cli"); // disable interrupts
        loop {
            asm!("hlt");  // halt
        }
    }
}

pub fn without_interrupts<R,F: FnOnce()->R>(f: F) -> R{
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(f)
}

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