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
        unsafe{memory::kernel_heap::init_kheap_2();}
        // Grow kernel heap by 16+8MiB for a total initial size of 32
        klog!(Info, BOOT, "Expanding kernel heap");
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

static AP_BOOT_PAGING_CONTEXT: sync::YMutex<Option<memory::paging::PagingContext>> = sync::YMutex::new(None);
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
    let mut writer = VGA_WRITER.lock();
    writer.write_string("OKAY!! 👌👌👌👌");
    
    writer.write_string("\n\nAccording to all known laws of aviation, there is no possible way for a bee to be able to fly. Its wings are too small to get its fat little body off the ground. The bee, of course, flies anyway, because bees don't care what humans think is impossible.");
    
    let s = format!("\n\nKernel bounds: {:x?}", memory::physical::get_kernel_bounds());
    writer.write_string(&s);
    drop(writer);
    
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
