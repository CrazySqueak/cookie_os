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
    writer.write_byte(b'K');
    
    // TODO
    loop {}
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}