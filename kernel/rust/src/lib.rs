#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![feature(box_into_inner)]
#![feature(vec_pop_if)]
#![feature(new_uninit)]
#![feature(asm_const)]

// i'm  exhausted by these warnings jeez
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

extern crate alloc;

pub mod memory;
pub mod multitasking;
pub mod sync;
pub mod coredrivers;

pub mod logging;
use logging::{klog,emergency_kernel_log};
pub mod panic;

// arch-specific code lives in "x::arch" for some modules
macro_rules! arch_specific_module {
    ($v:vis mod $name:ident) => {
        $v mod $name { cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                mod x86_64;
                pub use x86_64::*;
            } else {
                compile_error!(concat!("This architecture is unsupported as it does not have an implementation for the '",stringify!($name),"' module!"));
            }
        }}
    }
}
pub(crate) use arch_specific_module;

#[no_mangle]
pub extern "sysv64" fn _kstart() -> ! {
    {
    // Initialise heap
    unsafe { memory::kernel_heap::init_kheap(); }
    // Initialise Fixed CPU Locals
    multitasking::fixedcpulocal::init_fixed_cpu_locals();
    // LATE BOOTSTRAP - The bare minimum is ready for rust code to execute
    klog!(Info, BOOT, "COOKIE version 0.0.2");
    klog!(Info, BOOT, "\"Now with less asbestos!\"");
    klog!(Info, BOOT, "=========================");
    klog!(Info, MEMORY_KHEAP, "Kernel heap initialised with {} bytes.", unsafe{memory::kernel_heap::kheap_initial_size});
    // Initialise scheduler
    multitasking::scheduler::init_scheduler(None);
    
    // Configure physical memory
    klog!(Info, BOOT, "Initialising physical memory allocator...");
    let memmap = coredrivers::parse_multiboot::MULTIBOOT_MEMORY_MAP.expect("No memory map found!");
    memory::physical::init_pmem(memmap);
    // Configure virtual memory
    klog!(Info, BOOT, "Initialising virtual memory mappings...");
    let pagetable = memory::alloc_util::new_user_paging_context();
    unsafe{pagetable.activate()};
    }
    
    // TODO
    let x = alloc::format!("{:?}",multitasking::fixedcpulocal::get_fixed_cpu_locals().CURRENT_NOINTERRUPTIONS_STATE);
    klog!(Info, ROOT, &x);
    klog!(Info, BOOT, "Further boot process not yet implemented.");
    multitasking::terminate_current_task();
}
#[no_mangle]
pub extern "sysv64" fn _kstart_ap() -> ! {
    todo!()
}

// fixme
#[no_mangle]
#[used]
static next_processor_stack: u8 = 0xaa;