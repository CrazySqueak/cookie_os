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
use multitasking::util::def_task_fn;

// arch-specific code lives either in "lowlevel", or in "x::arch" for stuff specific to certain modules
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
// arch-specific "lowlevel" module
arch_specific_module!(mod lowlevel);
//#[cfg_attr(target_arch = "x86_64", path = "lowlevel/x86_64/mod.rs")]
//mod lowlevel;

#[no_mangle]
pub extern "sysv64" fn _kstart() -> ! {
    multitasking::init_cpu_num();
    {   // N.B. anything that is owned here gets dropped before _kmain is called.
        // This is useful because terminate_current_task doesn't perform unwinding or drop any values held by the given task.
        
        // Create initial heap
        unsafe{memory::kernel_heap::init_kheap();}
        
        // Initialise CPU/system
        lowlevel::init1_bsp();
        // BOOTSTRAP
        klog!(Info, BOOT, "Starting <OS_NAME> version <VERSION>...");
        // Initialise scheduler
        klog!(Info, BOOT, "Configuring scheduler");
        multitasking::scheduler::init_scheduler(None);
        
        // Initialise physical memory
        klog!(Info, BOOT, "Reading memory map");
        memory::physical::init_pmem(coredrivers::parse_multiboot::MULTIBOOT_MEMORY_MAP.expect("No memory map found!"));
        
        // Initialise paging
        klog!(Info, BOOT, "Configuring page tables");
        use alloc::boxed::Box;
        use memory::paging::{PagingContext,PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
        let pagetable = memory::alloc_util::new_user_paging_context();
        {
            let allocator = &pagetable;
            let kallocator = &memory::paging::global_pages::KERNEL_PTABLE;
            
            // Memory-map the MMIO we're using
            klog!(Info, BOOT, "Adding MMIO to page mappings");
            display_vga::map_vga_mmio().expect("Unable to map VGA buffer!").leak();
            system_apic::map_local_apic_mmio().expect("Unable to map local APIC buffer!").leak();
            
            // Guess who doesn't have to manually map the kernel in lib.rs anymore because it's done in global_pages.rs!!!
        }
        // Activate context
        klog!(Info, BOOT, "Activating kernel-controlled page tables");
        unsafe{pagetable.activate();}
        // LATE-BOOTSTRAP
        
        // Initialise CPU/system (part II)
        klog!(Info, BOOT, "Configuring CPU features and interrupts");
        lowlevel::init2_bsp();
        
        // Initialise kernel heap rescue
        klog!(Info, BOOT, "Expanding kernel heap");
        unsafe{memory::kernel_heap::init_kheap_2();}
        
        // Grow kernel heap by 16+8MiB for a total initial size of 32
        let _ = memory::kernel_heap::grow_kheap(16*1024*1024);
        let _ = memory::kernel_heap::grow_kheap( 8*1024*1024);
        
        // EARLY-MULTIPROGRAM
        klog!(Info, BOOT, "Entered multiprogram phase.");
        
        // Begin waking processors
        klog!(Info, BOOT, "Starting CPU cores (executing in background)");
        _start_processors_task::spawn();
        multitasking::yield_to_scheduler(multitasking::SchedulerCommand::PushBack);  // yield immediately since starting processors is I/O-bound and will yield to us pretty soon
    }
    
    // Call kmain
    klog!(Info, BOOT, "Bootstrapping complete. Executing _kmain().");
    _kmain();
}

static AP_BOOT_PAGING_CONTEXT: sync::Mutex<Option<memory::paging::PagingContext>> = sync::Mutex::new(None);
#[no_mangle]
pub extern "sysv64" fn _kstart_ap() -> ! {
    multitasking::init_cpu_num();
    klog!(Info, BOOT, "Secondary CPU initialising");
    {   // N.B. Everything used in initialisation should be dropped before the call to _apmain, because terminate_current_task doesn't perform unwinding
        
        // Signal that we've started
        //TODO//multitasking::scheduler::PROCESSORS_READY.fetch_add(1, Ordering::Acquire);
        // Take ownership of our bootstrap stack
        let bootstrap_stack = unsafe{lowlevel::_get_bootstrap_stack()};
        // EARLY-BOOTSTRAP
        
        // Initialise CPU/system
        lowlevel::init1_ap();
        // BOOTSTRAP
        // Initialise scheduler
        multitasking::scheduler::init_scheduler(bootstrap_stack);
        
        // Initialise paging
        // (the paging context is shared between CPUs to avoid allocating a new one every time)
        let page_table = AP_BOOT_PAGING_CONTEXT.lock().as_ref().map(memory::paging::PagingContext::clone_ref).unwrap_or_else(memory::alloc_util::new_user_paging_context);
        unsafe{page_table.activate();}
        drop(page_table);  // (ensure paging context gets dropped once it's no longer active)
        // LATE-BOOTSTRAP
        
        // Initialise CPU/system (part II)
        lowlevel::init2_ap();
        
        // EARLY-MULTIPROGRAM
    }
    klog!(Info, BOOT, "Secondary CPU successfully entered multiprogram phase. Executing _apmain().");
    
    // Call apmain since there's nothing else to do yet
    _apmain();
}

#[no_mangle]
pub fn _kmain() -> ! {
    VGA_WRITER.write_string("OKAY!! 👌👌👌👌");
    
    VGA_WRITER.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    let s = format!("\n\nKernel bounds: {:x?}", memory::physical::get_kernel_bounds());
    VGA_WRITER.write_string(&s);
    
    // test
    for i in 0..3 {
        //let kstack = memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
        //    let task = multitasking::Task::new_kernel_task(test, alloc::boxed::Box::new(kstack));
        //multitasking::scheduler::push_task(task);
        test_task::spawn(i*3 +2);
        
        multitasking::yield_to_scheduler(multitasking::SchedulerCommand::PushBack);
    }
    
    // TODO
    // For the love of god, please let other tasks run instead of blocking
    // pre-emption hasn't been implemented yet
    multitasking::terminate_current_task();
}

#[no_mangle]
pub fn _apmain() -> ! {
    multitasking::yield_to_scheduler(multitasking::SchedulerCommand::SleepNTicks(10));
    klog!(Info,ROOT,"Hello :)");
    multitasking::terminate_current_task();
}

def_task_fn!{ task fn test_task(x:usize) {
    for i in 0..x {
        klog!(Info,ROOT,"{}", i);
        multitasking::yield_to_scheduler(multitasking::SchedulerCommand::PushBack);
    }
}}

def_task_fn! {
 task fn _start_processors_task() {
    // Attempt to start all available processors on the system, one-by-one
    let our_apic_id = coredrivers::system_apic::get_apic_id_for(multitasking::get_cpu_num());
    
    // Parse ACPI tables
    use coredrivers::parse_acpi_tables;
    let Some(acpi_tables) = parse_acpi_tables::parse_tables_multiboot() else {
        klog!(Severe, BOOT, "Failed to parse ACPI tables: No RSDP found!");
        return;
    };
    let Ok(acpi_tables) = acpi_tables else {
        let Err(err) = acpi_tables else {unreachable!()};
        klog!(Severe, BOOT, "Failed to parse ACPI tables: Got Err({:?})!", err);
        return;
    };
    let    acpi_info  = acpi_tables.platform_info();
    let Ok(acpi_info) = acpi_info else {
        let Err(err) = acpi_info else {unreachable!()};
        klog!(Severe, BOOT, "Failed to parse ACPI tables: Got Err({:?})!", err);
        return;
    };
    let Some(processor_info) = acpi_info.processor_info else {
        klog!(Severe, BOOT, "No processor info found in ACPI tables!");
        return;
    };
    assert!(processor_info.boot_processor.local_apic_id == our_apic_id.into());
    
    // Set up a paging context
    let context = memory::alloc_util::new_user_paging_context();
    *AP_BOOT_PAGING_CONTEXT.lock() = Some(context);
    
    // Start the CPUs
    let mut num_started = 0; let mut num_skipped = 0; let mut num_failed = 0;
    for processor in processor_info.application_processors.iter() {
        let Ok(apic_id): Result<u8,_> = processor.local_apic_id.try_into()  else {
            klog!(Warning, BOOT, "Skipping CPU with APIC ID >255");
            num_skipped += 1;
            continue;
        };
        if let acpi::platform::ProcessorState::Disabled = processor.state {
            klog!(Warning, BOOT, "CPU with APIC ID {} is disabled. Skipping...", apic_id);
            num_skipped += 1;
            continue;
        }
        
        if let Ok(_) = unsafe{ lowlevel::start_processor_xapic(apic_id) } {
            num_started += 1;
        } else {
            klog!(Warning, BOOT, "CPU with APIC ID {} failed to start!", apic_id);
            num_failed += 1;
        }
    }
    
    // Drop the paging context
    *AP_BOOT_PAGING_CONTEXT.lock() = None;
    
    // Now terminate
    klog!(Info, BOOT, "Started {} secondary CPUs. ({} failed, {} skipped)", num_started, num_failed, num_skipped);
    return;
 }
}

/// This function is called on panic.
/// If the task that triggers a panic is not a bootstrap or scheduler task, then the task terminates and a log message is printed. (this is still a serious issue, however, as a kernel-level panic at the wrong time could lock up the system - kernel panics should represent errors that present immediate threat to the operating system e.g. where locking up the whole system would be a viable handling strategy)
/// Otherwise, a kernel panic occurs and the whole thing goes tits-up
use crate::logging::emergency_kernel_log;

use core::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
// This variable tracks if we are already aborting due to a panic
// If this is true when a panic occurs, the panic handler simply halts (detecting that an infinite panic loop has occurred)
static _ABORTING: AtomicBool = AtomicBool::new(false);
// To avoid contention issues but keep the utility, only the panicking CPU will print the "end of panic" messages
static _PANICKING_CPU: AtomicUsize = AtomicUsize::new(0xFF69420101);  // whatever value I put here as the default won't be read anyway, so might as well make it something significant
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    use core::fmt::Write;
    let cpu_num = multitasking::get_cpu_num();
    if _ABORTING.load(Ordering::Acquire) {
        // A panic has occurred already, and we are panicking now
        // This probably means that we are either shutting down or in an infinite panic loop.
        // oh well
        if _PANICKING_CPU.load(Ordering::Acquire) == cpu_num.into() {
            emergency_kernel_log!("\r\nAborting kernel panic handler due to secondary panic: {}\r\nYou're on your own from here.\r\n", _info);
        }
        //emergency_kernel_log!("\r\nCPU {} now aborting due to secondary panic: {}.\r\n", cpu_num, _info.message());
        lowlevel::halt();
    }
    // Otherwise, begin panic
    if cfg!(feature = "recover_from_task_related_kernel_panic") && multitasking::is_executing_task() {
        // Terminate current task
        // This is still a serious issue - as we do not unwind the stack, the whole system could lock up
        // Additionally, such an issue could hint at underlying bugs or system instability
        let context = multitasking::ExecutionContext::current();
        klog!(Severe, ROOT, "KERNEL PANIC at {} (task terminated): {}", context, _info);
        multitasking::terminate_current_task();
    } else {
        // Kernel panic - halt and catch fire, performing operations in order of priority, with high-risk operations (e.g. heap allocations, mmio) happening after high-priority low-risk operations.
        // First, print an early warning to serial to ensure the error message is printed
        // Stack allocation, force unlocks the serial - low risk
        emergency_kernel_log!("\r\n\r\n*** KERNEL PANIC on CPU {} (unrecoverable): {}\r\n", cpu_num, _info);
        // Set aborting to true to detect any panic loops - no risk
        _ABORTING.store(true, Ordering::SeqCst);
        _PANICKING_CPU.store(cpu_num.into(), Ordering::SeqCst);
        
        // Begin shutting down CPUs - requires MMIO to be mapped for APIC to work - med risk but high importance. paging/MMIO is mapped way before multitasking is configured anyway
        unsafe{lowlevel::emit_panic();}
        
        // Print more debug information - requires scheduler to be in a sane state (either there or not there) - med risk
        let context = multitasking::ExecutionContext::current();
        emergency_kernel_log!("Execution Context: {}\r\n", context);
        
        // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
        // Requires MMIO to be mapped - med risk
        let mut writer = unsafe{let wm=core::mem::transmute::<&display_vga::LockedVGAConsoleWriter,&crate::sync::Mutex<display_vga::VGAConsoleWriter>>(&*VGA_WRITER);wm.force_unlock();wm.lock()};
        writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
        // Write message and location to screen
        let _ = write!(writer, "\n\nKERNEL PANICKED (@{}): {}", context, _info);
        
        // Attempt to perform backtrace
        
        // Finally, halt
        emergency_kernel_log!("\r\nEnd of kernel panic. Halting.");
        lowlevel::halt();
    }
}
