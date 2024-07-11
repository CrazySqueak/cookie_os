use core::arch::asm;

/* The offset between the kernel's virtual memory and the computer's physical memory.
    As this is a higher-half kernel, any memory mapped I/O, page table locations, etc. should be converted using this constant.
    (the kernel will always be mapped between the given physical memory and virtual memory)
    Note that converting userspace addresses by this constant will not end well, as they are mapped by their page table (and are not necessarily contiguous in physical memory.) */
pub const HIGHER_HALF_OFFSET: usize = 0xFFFF800000000000;

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
