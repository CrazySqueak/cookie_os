
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
pub mod context_switch;  // testing

pub use lowlevel::{halt, without_interrupts};

/* Early initialisation prior to paging/extendedheap/etc. setup. */
pub fn init() {
    lowlevel::init_msr();
    gdt::init();
    interrupts::init();
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