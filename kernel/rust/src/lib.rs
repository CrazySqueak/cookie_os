#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

use core::panic::PanicInfo;
use alloc::format;

use buddy_system_allocator::LockedHeap;

mod vga_buffer;
use vga_buffer::VGA_WRITER;
mod serial;
use serial::SERIAL1;
mod util;
use crate::util::LockedWrite;

mod coredrivers;

// arch-specific "lowlevel" module
#[cfg_attr(target_arch = "x86_64", path = "lowlevel/x86_64/mod.rs")]
mod lowlevel;

pub fn _kinit() {
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
    
    lowlevel::init();
}

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    _kinit();
    
    VGA_WRITER.write_string("OKAY!! 👌👌👌👌");
    
    VGA_WRITER.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    for i in 1..11 {
        let s = format!("\n\nBeep {}",i);
        VGA_WRITER.write_string(&s);
    }
    
    for entry in unsafe { (*lowlevel::multiboot::multiboot_info_ptr).get_memmap().unwrap() } {
        VGA_WRITER.write_string(&format!("\n{:?}",entry));
    }
    
    let _ = write!(SERIAL1, "Hello World!");
    
    // TODO
    loop{}//lowlevel::halt();
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let mut writer = vga_buffer::VGAConsoleWriter::new_with_buffer(vga_buffer::get_standard_vga_buffer());  // we can't lock the normal writer as it may be already held by the current thread, which would cause a deadlock
    writer.set_colour(vga_buffer::VGAColour::new(vga_buffer::BaseColour::LightGray,vga_buffer::BaseColour::Red,true,false));
    
    // Write message and location
    let _ = writer.write_string(&format!("KERNEL PANICKED: {}", _info));
    // Write to serial as well
    let _ = write!(SERIAL1, "Kernel Rust-Panic!: {}", _info);
    
    lowlevel::halt();
}

#[global_allocator]
static ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();