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

mod memory;
mod descriptors;
mod scheduler;

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
    
    // Initialise paging
    use alloc::boxed::Box;
    use memory::paging::{PagingContext,PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
    let pagetable = memory::alloc_util::new_user_paging_context();
    {
        let allocator = &pagetable;
        let kallocator = &memory::paging::global_pages::KERNEL_PTABLE;
        
        // VGA Buffer memory-mapped IO
        let vgabuf = kallocator.allocate_at(display_vga::VGA_BUFFER_ADDR, display_vga::VGA_BUFFER_SIZE).expect("Unable to map VGA buffer");
        vgabuf.set_base_addr(display_vga::VGA_BUFFER_PHYSICAL, PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::PINNED));
        vgabuf.leak();
        
        // Guess who doesn't have to manually map the kernel in lib.rs anymore because it's done in global_pages.rs!!!
    }
    // Activate context
    pagetable.activate();
    
    // Initialise kernel heap rescue
    memory::kernel_heap::init_kheap_2();
    
    // Grow kernel heap by 16+8MiB for a total initial size of 32
    let _ = memory::kernel_heap::grow_kheap(16*1024*1024);
    let _ = memory::kernel_heap::grow_kheap( 8*1024*1024);
    
    // Initialise low-level function (part 2 now that memory is configured)
    lowlevel::init2();
    
    // Initialise scheduler
    scheduler::context_switch::init_scheduler();
    
    // Wake up CPUs (in a background task)
    // TODO: create spawn_task function
    {
        let kstack = memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
        let rsp = unsafe { lowlevel::context_switch::_cs_new(start_aps, kstack.bottom_vaddr() as *const u8) };
        let task = unsafe { scheduler::Task::new_with_rsp(scheduler::TaskType::KernelTask, rsp) };
        scheduler::context_switch::push_task(task);
        core::mem::forget(kstack);  // I keep forgetting to NOT DROP THE STACK - i really need to put together a proper API for managing this
    }
}

extern "sysv64" fn start_aps() -> ! {
    // TODO: Find a way to identify the number and IDs of APs
    unsafe {
        scheduler::multicore::start_processor(1);
        scheduler::multicore::start_processor(2);
        scheduler::multicore::start_processor(3);
    }
    
    // Done :)
    scheduler::terminate_current_task();
}

#[no_mangle]
pub extern "sysv64" fn _kmain() -> ! {
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
        let task = unsafe { scheduler::Task::new_with_rsp(scheduler::TaskType::KernelTask, rsp) };
        scheduler::context_switch::push_task(task);
        
        scheduler::yield_to_scheduler(scheduler::SchedulerCommand::PushBack);
        
        // ALSO FOR THE LOVE OF GOD
        // DON'T DROP() THE STACK WHILE YOU'RE STILL USING IT
        // DUMBASS
        core::mem::forget(kstack);
    }
    
    // TODO
    loop{}//lowlevel::halt();
}

use core::sync::atomic::{AtomicPtr,Ordering};
#[no_mangle]
pub extern "sysv64" fn _kapstart() -> ! {
    // Signal that we've started
    scheduler::multicore::PROCESSORS_READY.fetch_add(1, Ordering::Acquire);
    
    // Load page table
    unsafe { let context = memory::alloc_util::new_user_paging_context(); context.activate(); }
    
    // TODO: Init CPU?
    klog!(Info,ROOT,"Hello :)");
    loop{};
}

extern "sysv64" fn test() -> ! {
    for i in 0..5 {
        klog!(Info,ROOT,"{}", i);
        scheduler::yield_to_scheduler(scheduler::SchedulerCommand::PushBack);
    }
    scheduler::terminate_current_task();
}

/// This function is called on panic.
/// If the task that triggers a panic is not a bootstrap or scheduler task, then the task terminates and a log message is printed. (this is still a serious issue, however, as a kernel-level panic at the wrong time could lock up the system - kernel panics should represent errors that present immediate threat to the operating system e.g. where locking up the whole system would be a viable handling strategy)
/// Otherwise, a kernel panic occurs and the whole thing goes tits-up
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let panic_header = format!("KERNEL PANIC on CPU {} in task {}", lowlevel::get_cpu_id(), scheduler::context_switch::get_task_id());
    if scheduler::is_scheduler_ready() {
        // Terminate current task
        // This is still a serious issue - as we do not unwind the stack, the whole system could lock up
        // Additionally, such an issue could hint at underlying bugs or system instability
        klog!(Severe, ROOT, "{} (task terminated): {}", panic_header, _info);
        scheduler::terminate_current_task();
    } else {
        // Kernel panic
        klog!(Fatal, ROOT, "{} (unrecoverable): {}", panic_header, _info);
        
        // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
        let mut writer = unsafe{let wm=core::mem::transmute::<&display_vga::LockedVGAConsoleWriter,&crate::sync::Mutex<display_vga::VGAConsoleWriter>>(&*VGA_WRITER);wm.force_unlock();wm.lock()};
        writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
        
        // Write message and location
        let _ = writer.write_string(&format!("\n\n\n\n\nKERNEL PANICKED (CPU {}): {}", lowlevel::get_cpu_id(), _info));
        
        lowlevel::halt();
    }
}
