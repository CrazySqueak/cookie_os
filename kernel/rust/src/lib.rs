#![no_std]
use core::panic::PanicInfo;

mod vga_buffer;

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    //unsafe {
    //    let vga_ok: u64 = 0x2f592f412f4b2f4f;
    //    let vga_ptr: *mut u64 = 0xb8000 as *mut u64;
    //    (*vga_ptr) = vga_ok
    //}
    let mut writer = vga_buffer::VGAConsoleWriter::new();
    writer.write_string("OKAY!! 👌👌👌👌");
    
    writer.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    for i in 1..10 {
        writer.write_string("\n\nBeep");
    }
    
    // TODO
    loop {}
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
