#![no_std]
extern crate alloc;

use core::panic::PanicInfo;
use core::fmt::Write;
use alloc::format;

use buddy_system_allocator::LockedHeap;

mod vga_buffer;
use vga_buffer::VGA_WRITER;
mod serial;
use serial::SERIAL1;

// arch-specific
mod lowlevel;

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    // Init heap
    // TODO: page this and properly configure it
    unsafe {
        ALLOCATOR.lock().init(0x800_000,128*1024);  // TODO: figure out wtf is going on here this heap is haunted ISTG
        // At first I thought it was an issue with my paging setup but the paging seemed fine (well, as fine as I could tell - couldn't get a sensible disassembly out of gdb which made life a pain (movabs with a 64-bit operand in protected mode????? the fuck are you smoking?))
        // and after testing, a paging issue would cause a triple-fault instead (since I don't have a handler for page faults nor for double faults yet)
        // So uhh
        // Idk
        // QEMU bug? Some issue with my own code that i haven't figured out yet?
        // if I init() with 128*1024, it's fine, but if I init() with 128*1024*1024, the heap appears to be write-only
        // ????????????
        // TODO: Perform an exorcism
    }
    
    //unsafe {
    //    let vga_ok: u64 = 0x2f592f412f4b2f4f;
    //    let vga_ptr: *mut u64 = 0xb8000 as *mut u64;
    //    (*vga_ptr) = vga_ok
    //}
    let mut writer = VGA_WRITER.lock();
    writer.write_string("OKAY!! 👌👌👌👌");
    
    writer.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    for i in 1..11 {
        let s = format!("\n\nBeep {}",i);
        writer.write_string(&s);
    }
    
    
    write!(SERIAL1.lock(), "Hello World!");
    
    // TODO
    lowlevel::halt();
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let mut writer = vga_buffer::VGAConsoleWriter::new_with_buffer(vga_buffer::get_standard_vga_buffer());  // we can't lock the normal writer as it may be already held by the current thread, which would cause a deadlock
    writer.set_colour(vga_buffer::VGAColour::new(vga_buffer::BaseColour::LightGray,vga_buffer::BaseColour::Red,true,false));
    
    // Write message and location
    writer.write_string(&format!("KERNEL PANICKED: {}", _info));
    // Write to serial as well
    write!(SERIAL1.lock(), "Kernel Rust-Panic!: {}", _info);
    
    lowlevel::halt();
}

#[global_allocator]
static ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();