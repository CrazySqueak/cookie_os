#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]

// i'm  exhausted by these warnings jeez
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

extern crate alloc;
use core::panic::PanicInfo;
use alloc::format;

mod sync;
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

// Like all init functions, this must be called ONCE. No more. No less.
pub unsafe fn _kinit() {
    // Create initial heap
    memory::kernel_heap::init_kheap();
    
    // Initialise low-level functions
    lowlevel::init();
    
    // Initialise physical memory
    memory::physical::init_pmem(lowlevel::multiboot::MULTIBOOT_MEMORY_MAP.expect("No memory map found!"));
    
    // Initialise paging (bunch of testing code)
    use alloc::boxed::Box;
    use memory::paging::{PagingContext};
    let pagetable = PagingContext::new();
    {
        let mut allocator = pagetable.write();
        let mut kallocator = memory::paging::global_pages::KERNEL_PTABLE.write_when_active();
        let (start, size) = (0, 1*1024*1024*1024);  // 1GiB - currently akin to the bootstrap page table
        
        // Null guard
        let nullguard = allocator.allocate_at(0, 1).expect("VMem Allocation Failed!");
        allocator.set_absent(&nullguard, 0x4E554C_505452);  // "NULPTR"
        
        // From start -> stack guard
        let alloc1 = kallocator.allocate_at(start+lowlevel::HIGHER_HALF_OFFSET, size).expect("VMem Allocation Failed!");
        use memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
        kallocator.set_base_addr(&alloc1, 0, PageFlags::new(TransitivePageFlags::EXECUTABLE /* TODO */, MappingSpecificPageFlags::PINNED)); // 0+HHOFF -> 0
        let alloc2 = kallocator.allocate(4096).expect("Test alloc failed!");
        kallocator.set_base_addr(&alloc2, 0, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()));
        
        // // test 2
        // let at2 = kallocator.allocate(2*1024*1024*1024 + 512*1024).unwrap();
        // kallocator.set_base_addr(&at2, 1207*4096, PageFlags::new(TransitivePageFlags::USER_WRITEABLE, MappingSpecificPageFlags::empty()));
        
        // test 3
        //for split in [2*1024*1024, 4096] {
        //    let at3 = allocator.allocate(2*1024*1024 +1).unwrap();
        //    klog!(Info, ROOT, "Splitting {:?} at {:x}", at3, split);
        //    let (at3l, at3r) = allocator.split_allocation(at3, split);
        //    klog!(Info, ROOT, "Got {:?}, {:?}", at3l, at3r);
        //    allocator.set_absent(&at3l, 0xF47B33F0);
        //    allocator.set_base_addr(&at3r, 0xFFFF * 4096, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty()));
        //}
    }
    // Activate context
    pagetable.activate();
    
    // Initialise kernel heap rescue
    memory::kernel_heap::init_kheap_2();
    
    // test kernel heap rescue
    let mut i = 0;
    loop {
        i+=1;
        const L: alloc::alloc::Layout = unsafe{alloc::alloc::Layout::from_size_align_unchecked(69420,2)};
        let p = alloc::alloc::alloc(L);
        if p == core::ptr::null_mut() { alloc::alloc::handle_alloc_error(L); }
        klog!(Info,ROOT,"{} {:p}",i,p);
    }
    
    
    // Grow kernel heap by 16+32MiB
    //let _ = memory::kernel_heap::grow_kheap(16*1024*1024);
    //let _ = memory::kernel_heap::grow_kheap(32*1024*1024);
}

#[no_mangle]
pub extern "C" fn _kmain() -> ! {
    unsafe{_kinit();}
    
    VGA_WRITER.write_string("OKAY!! 👌👌👌👌");
    
    VGA_WRITER.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    let s = format!("\n\nKernel bounds: {:x?}", memory::physical::get_kernel_bounds());
    VGA_WRITER.write_string(&s);
    
    unsafe { lowlevel::context_switch::_cs_push(test); }
    
    // TODO
    loop{}//lowlevel::halt();
}

extern "sysv64" fn test(rsp: *const u8) -> ! {
    klog!(Info, ROOT, "RSP: {:p}", rsp);
    unsafe { lowlevel::context_switch::_cs_pop(rsp); }
}

/// This function is called on panic.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    klog!(Fatal, ROOT, "KERNEL PANIC: {}", _info);
    
    // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
    let mut writer = unsafe{let wm=core::mem::transmute::<&display_vga::LockedVGAConsoleWriter,&crate::sync::Mutex<display_vga::VGAConsoleWriter>>(&*VGA_WRITER);wm.force_unlock();wm.lock()};
    writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
    
    // Write message and location
    let _ = writer.write_string(&format!("\n\n\n\n\nKERNEL PANICKED: {}", _info));
    
    lowlevel::halt();
}
