use core::arch::asm;

mod gdt;
mod interrupts;

pub fn init() {
    gdt::init();
    interrupts::init();
}

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
