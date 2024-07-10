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

extern "C" {
    // Provided by boot.intel.asm
    pub static kheap_initial_addr: u32;
    pub static kheap_initial_size: u32;
}

pub fn _kinit() {
    // Init heap
    // TODO: page this and properly configure it
    unsafe {
        ALLOCATOR.lock().init(kheap_initial_addr as usize,kheap_initial_size as usize);
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
    
    //VGA_WRITER.write_string(&format!("\n{:?}",*lowlevel::multiboot::MULTIBOOT_TAGS));
    let _ = write!(SERIAL1, "\nTags={:?}",*lowlevel::multiboot::MULTIBOOT_TAGS);
    let _ = write!(SERIAL1, "\nMemMap={:#?}",*lowlevel::multiboot::MULTIBOOT_MEMORY_MAP);
    
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