#![no_std]
#![feature(abi_x86_interrupt)]
extern crate alloc;

use core::panic::PanicInfo;
use alloc::format;

use buddy_system_allocator::LockedHeap;

mod util;
use crate::util::{LockedNoInterrupts,LockedWrite};

mod coredrivers;
use coredrivers::serial_uart::SERIAL1;
use coredrivers::display_vga; use display_vga::VGA_WRITER;

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
    VGA_WRITER.with_lock(|mut w|panic!("test"));
    loop{}//lowlevel::halt();
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
    let mut writer = unsafe{let wm=core::mem::transmute::<&display_vga::LockedVGAConsoleWriter,&spin::Mutex<display_vga::VGAConsoleWriter>>(&*VGA_WRITER);wm.force_unlock();wm.lock()};
    writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
    
    // Write message and location
    let _ = writer.write_string(&format!("\n\n\n\n\nKERNEL PANICKED: {}", _info));
    // Write to serial as well
    let _ = write!(SERIAL1, "\n\n\n\n\nKernel Rust-Panic!: {}", _info);
    
    lowlevel::halt();
}

#[global_allocator]
static ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();