#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![feature(box_into_inner)]

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
use coredrivers::system_apic;

mod memory;
mod descriptors;
mod multitasking;

// arch-specific "lowlevel" module
#[cfg_attr(target_arch = "x86_64", path = "lowlevel/x86_64/mod.rs")]
mod lowlevel;

#[used]
#[no_mangle]
static next_processor_stack: u8 = 0;  // TODO

// Like all init functions, this must be called ONCE. No more. No less.
pub unsafe fn _kinit() {
    // Create initial heap
    memory::kernel_heap::init_kheap();
    
    // Initialise low-level functions
    lowlevel::init();
    
    // Initialise physical memory
    memory::physical::init_pmem(lowlevel::multiboot::MULTIBOOT_MEMORY_MAP.expect("No memory map found!"));
    
    // Initialise paging
    use alloc::boxed::Box;
    use memory::paging::{PagingContext,PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
    let pagetable = memory::alloc_util::new_user_paging_context();
    {
        let allocator = &pagetable;
        let kallocator = &memory::paging::global_pages::KERNEL_PTABLE;
        
        // Memory-map the MMIO we're using
        display_vga::map_vga_mmio().expect("Unable to map VGA buffer!").leak();
        system_apic::map_local_apic_mmio().expect("Unable to map local APIC buffer!").leak();
        
        // Guess who doesn't have to manually map the kernel in lib.rs anymore because it's done in global_pages.rs!!!
    }
    // Activate context
    pagetable.activate();
    
    // Initialise kernel heap rescue
    memory::kernel_heap::init_kheap_2();
    
    // Grow kernel heap by 16+8MiB for a total initial size of 32
    let _ = memory::kernel_heap::grow_kheap(16*1024*1024);
    let _ = memory::kernel_heap::grow_kheap( 8*1024*1024);
    
    // Initialise scheduler
    multitasking::scheduler::init_scheduler();
}

#[no_mangle]
pub extern "sysv64" fn _kmain() -> ! {
    multitasking::init_cpu_num();
    unsafe{_kinit();}
    
    VGA_WRITER.write_string("OKAY!! 👌👌👌👌");
    
    VGA_WRITER.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    let s = format!("\n\nKernel bounds: {:x?}", memory::physical::get_kernel_bounds());
    VGA_WRITER.write_string(&s);
    
    // test
    for i in 0..3 {
        let kstack = memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
        let rsp = unsafe { lowlevel::context_switch::_cs_new(test, kstack.bottom_vaddr() as *const u8) };
        klog!(Info,ROOT,"newtask RSP={:p}", rsp);
        let task = unsafe { multitasking::Task::new_with_rsp(multitasking::TaskType::KernelTask, rsp) };
        multitasking::scheduler::push_task(task);
        
        multitasking::yield_to_scheduler(multitasking::SchedulerCommand::PushBack);
        
        // ALSO FOR THE LOVE OF GOD
        // DON'T DROP() THE STACK WHILE YOU'RE STILL USING IT
        // DUMBASS
        core::mem::forget(kstack);
    }
    
    // TODO
    // For the love of god, please let other tasks run instead of blocking
    // pre-emption hasn't been implemented yet
    multitasking::terminate_current_task();
}

use core::sync::atomic::{AtomicPtr,Ordering};
#[no_mangle]
pub extern "sysv64" fn _kapstart() -> ! {
    multitasking::init_cpu_num();
    // Signal that we've started
    todo!();//multitasking::scheduler::PROCESSORS_READY.fetch_add(1, Ordering::Acquire);
    
    // TODO: Init CPU?
    //klog!(Info,ROOT,"Hello :)");
    //loop{};
}

extern "sysv64" fn test() -> ! {
    for i in 0..5 {
        klog!(Info,ROOT,"{}", i);
        multitasking::yield_to_scheduler(multitasking::SchedulerCommand::PushBack);
    }
    multitasking::terminate_current_task();
}

/// This function is called on panic.
/// If the task that triggers a panic is not a bootstrap or scheduler task, then the task terminates and a log message is printed. (this is still a serious issue, however, as a kernel-level panic at the wrong time could lock up the system - kernel panics should represent errors that present immediate threat to the operating system e.g. where locking up the whole system would be a viable handling strategy)
/// Otherwise, a kernel panic occurs and the whole thing goes tits-up
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let context = multitasking::ExecutionContext::current();
    let panic_header = format!("KERNEL PANIC at {}", context);
    if cfg!(feature = "recover_from_task_related_kernel_panic") && multitasking::is_executing_task() {
        // Terminate current task
        // This is still a serious issue - as we do not unwind the stack, the whole system could lock up
        // Additionally, such an issue could hint at underlying bugs or system instability
        klog!(Severe, ROOT, "{} (task terminated): {}", panic_header, _info);
        multitasking::terminate_current_task();
    } else {
        // Kernel panic
        klog!(Fatal, ROOT, "{} (unrecoverable): {}", panic_header, _info);
        
        // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
        let mut writer = unsafe{let wm=core::mem::transmute::<&display_vga::LockedVGAConsoleWriter,&crate::sync::Mutex<display_vga::VGAConsoleWriter>>(&*VGA_WRITER);wm.force_unlock();wm.lock()};
        writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
        
        // Write message and location
        let _ = writer.write_string(&format!("\n\n\n\n\nKERNEL PANICKED (CPU {}): {}", context.cpu_id, _info));
        
        lowlevel::halt();
    }
}
