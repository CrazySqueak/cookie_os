#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]
#![feature(const_for)]

extern crate alloc;
use core::panic::PanicInfo;
use alloc::format;

mod logging;
use logging::klog;
mod util;
use crate::util::{LockedWrite};

mod coredrivers;
use coredrivers::serial_uart::SERIAL1;
use coredrivers::display_vga; use display_vga::VGA_WRITER;

mod memory;

// arch-specific "lowlevel" module
#[cfg_attr(target_arch = "x86_64", path = "lowlevel/x86_64/mod.rs")]
mod lowlevel;

pub fn _kinit() {
    // Create initial heap
    memory::kernel_heap::init_kheap();
    
    // Initialise low-level functions
    lowlevel::init();
    
    // Initialise physical memory
    memory::physical::init_pmem(lowlevel::multiboot::MULTIBOOT_MEMORY_MAP.expect("No memory map found!"));
    
    // Grow kernel heap by 48MiB
    memory::kernel_heap::grow_kheap(48*1024*1024);
}

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    _kinit();
    
    VGA_WRITER.write_string("OKAY!! 👌👌👌👌");
    
    VGA_WRITER.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    let s = format!("\n\nKernel bounds: {:x?}", memory::physical::get_kernel_bounds());
    VGA_WRITER.write_string(&s);
    
    // Test allocation
    for i in 1..10 {
        let l = alloc::alloc::Layout::from_size_align(1024*i, 2048).unwrap();
        let x = memory::physical::palloc(l);
        klog!(Debug, "memory.physical", "Allocated {:?}, Got {:?}\n", l, x);
    }
    
    klog!(Debug, "beep", "Test1235");
    klog!(Debug, "beep", "2+2={}", 5);
    klog!(Warning, "beep", "Wait no");
    
    // TODO
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
