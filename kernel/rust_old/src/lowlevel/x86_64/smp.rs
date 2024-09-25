
use core::sync::atomic::{AtomicU16,AtomicU64,AtomicBool,AtomicPtr,Ordering};
use crate::memory::alloc_util::{AnyAllocatedStack,GAllocatedStack};
use crate::logging::klog;
use super::system_apic;

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

/// Address of the bootstrap stack allocation itself (taken by the bootstrapping CPU later on)
static BOOTSTRAP_STACK_ALLOCATION: AtomicPtr<GAllocatedStack> = AtomicPtr::new(core::ptr::null_mut());

/* Start the requested processor using the xAPIC. Blocks until this has completed.
    Note: This function is not re-entrant.*/
pub unsafe fn start_processor_xapic(target_apic_id: system_apic::ApicID) -> Result<(),()> {
    use crate::multitasking::{yield_to_scheduler,SchedulerCommand};
    klog!(Info, CPU_MANAGEMENT_SMP, "Starting CPU with APIC ID {}", target_apic_id);
    // Allocate stack
    let stack = alloc::boxed::Box::new(GAllocatedStack::allocate_kboot().ok_or(())?);
    next_processor_stack.store(stack.bottom_vaddr().try_into().unwrap(), Ordering::Release);
    BOOTSTRAP_STACK_ALLOCATION.store(alloc::boxed::Box::into_raw(stack), Ordering::Release);  // (we store the allocation itself separately as next_processor_stack is used when no stack is available yet, whereas BOOTSTRAP_STACK_ALLOCATION is used once safely in Rust code)
    
    // Send INIT-SIPI-SIPI
    let ipi_destination = system_apic::IPIDestination::APICId(target_apic_id);
    system_apic::with_local_apic(|apic|{
        let mut icr = apic.icr.lock();
        klog!(Debug, CPU_MANAGEMENT_SMP, "Sending INIT to APIC ID {}", target_apic_id);
        // Send INIT
        icr.send_ipi(system_apic::InterProcessorInterrupt::INIT, ipi_destination);
        // Wait for CPU to initialise
        yield_to_scheduler(SchedulerCommand::SleepNTicks(20));
        
        // Send up to 3 SIPIs (usually takes 2, sometimes takes 1. should never take 3)
        // until the trampoline code has started
        let mut num_sent: u8 = 0;
        let prev_processors_started = processors_started.load(Ordering::SeqCst);
        loop {
            klog!(Debug, CPU_MANAGEMENT_SMP, "Sending SIPI #{} to APIC ID {}", num_sent+1, target_apic_id);
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
    while next_processor_stack.load(Ordering::Relaxed) != 0 { yield_to_scheduler(SchedulerCommand::PushBack) };
    while BOOTSTRAP_STACK_ALLOCATION.load(Ordering::Relaxed) != core::ptr::null_mut() { yield_to_scheduler(SchedulerCommand::PushBack) };
    // The stack has been taken, so it's now owned by the task we gave it to
    
    // OK!
    Ok(())
}

/// take ownership of the stack saved by start_processor_xapic
/// This function itself is safe, however dropping the stack while it's still in use is undefined behaviour (which is why this function is unsafe)
pub unsafe fn get_bootstrap_stack() -> Option<alloc::boxed::Box<dyn AnyAllocatedStack + 'static>> {
    let ptr = BOOTSTRAP_STACK_ALLOCATION.swap(core::ptr::null_mut(), Ordering::AcqRel);
    if ptr == core::ptr::null_mut() { return None; }
    let stack = alloc::boxed::Box::from_raw(ptr);
    Some(stack)
}

/// Send a Kernel Panic interrupt to all other CPUs, bringing them down
pub unsafe fn emit_panic() {
    system_apic::with_local_apic(|apic|{
        unsafe{apic.icr.force_unlock();}
        let mut icr = apic.icr.lock();
        icr.send_ipi_raw(system_apic::InterProcessorInterrupt::Fixed(super::interrupts::KERNEL_PANIC_VECTOR), system_apic::IPIDestination::EveryoneButSelf);
    });
}