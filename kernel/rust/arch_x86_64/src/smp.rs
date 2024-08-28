
use core::sync::atomic::{AtomicU16,AtomicU64,Ordering};
use crate::memory::alloc_util::GAllocatedStack;
use crate::logging::klog;

extern "sysv64" {
    /// Number of APs that have started. Starts at 1.
    static processors_started: AtomicU16;
    /// The ap_trampoline_realmode function is the one started on the APs
    fn ap_trampoline_realmode() -> !;
}
/// Address of the next stack for use by bootstrapping CPUs
#[used]
#[no_mangle]
static next_processor_stack: AtomicU64 = AtomicU64::new(0);

/* Start the requested processor using the xAPIC. Blocks until this has completed.
    Note: This function is not re-entrant.*/
pub unsafe fn start_processor_xapic(target_apic_id: crate::coredrivers::system_apic::ApicID) -> Result<(),()> {
    use crate::coredrivers::system_apic;
    use crate::multitasking::{yield_to_scheduler,SchedulerCommand};
    klog!(Info, CPU_MANAGEMENT, "Starting CPU with APIC ID {}", target_apic_id);
    // Allocate stack
    let stack = GAllocatedStack::allocate_kboot().ok_or(())?;
    next_processor_stack.store(stack.bottom_vaddr().try_into().unwrap(), Ordering::SeqCst);
    
    // Send INIT-SIPI-SIPI
    let ipi_destination = system_apic::IPIDestination::APICId(target_apic_id);
    system_apic::with_local_apic(|apic|{
        let mut icr = apic.icr.lock();
        klog!(Debug, CPU_MANAGEMENT, "Sending INIT to APIC ID {}", target_apic_id);
        // Send INIT
        icr.send_ipi(system_apic::InterProcessorInterrupt::INIT, ipi_destination);
        // Wait for CPU to initialise
        yield_to_scheduler(SchedulerCommand::SleepNTicks(20));
        
        // Send up to 3 SIPIs (usually takes 2, sometimes takes 1. should never take 3)
        // until the trampoline code has started
        let mut num_sent: u8 = 0;
        let prev_processors_started = processors_started.load(Ordering::SeqCst);
        loop {
            klog!(Debug, CPU_MANAGEMENT, "Sending SIPI #{} to APIC ID {}", num_sent+1, target_apic_id);
            // Send SIPI
            icr.send_ipi(system_apic::InterProcessorInterrupt::SIPI(ap_trampoline_realmode as usize), ipi_destination);
            // Wait for CPU to boot
            yield_to_scheduler(SchedulerCommand::SleepNTicks(1));
            // Check processors_started
            num_sent += 1;
            if processors_started.load(Ordering::SeqCst) > prev_processors_started { break; }  // success
            if num_sent >= 3 { return Err(()); }  // failed
            // otherwise, try again
        };
        
        Ok(())
    })?;
    
    // Wait for stack to be taken
    while next_processor_stack.load(Ordering::Relaxed) != 0 { yield_to_scheduler(SchedulerCommand::PushBack)};
    // The stack has been taken, so it's now owned by the task we gave it to
    core::mem::forget(stack);  // leak the stack as we have no way to deallocate it when that core's bootstrap task quits
    
    // OK!
    Ok(())
}

/* Attempt to start all available processors on the system. Any which fail to start are skipped. */
pub unsafe fn start_all_processors_xapic_acpi(){
    use crate::coredrivers::system_apic;
    let our_apic_id = system_apic::get_apic_id_for(crate::multitasking::get_cpu_num());
    
    // TODO: figure out number of processors and their APIC IDs
    for i in 0..16 {
        if i == our_apic_id { continue; }
        let result = start_processor_xapic(i);
        match result {
            Ok(_) => {},
            Err(_) => klog!(Warning, CPU_MANAGEMENT, "CPU with APIC ID {} failed to start!", i),
        }
    }
    
    // Done :)
}