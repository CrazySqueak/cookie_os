#![no_std]

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    unsafe {
        let vga_ok: u64 = 0x2f592f412f4b2f4f;
        let vga_ptr: *mut u64 = 0xb8000 as *mut u64;
        (*vga_ptr) = vga_ok
    }
    
    // TODO
    loop {}
}

use core::panic::PanicInfo;

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}