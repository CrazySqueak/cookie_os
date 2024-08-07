use core::sync::atomic::{AtomicU16,Ordering,AtomicBool};
use x86_64::registers::model_specific::Msr;
use core::ptr::write_volatile;
use crate::memory::paging::global_pages::{KERNEL_PTABLE,KERNEL_PTABLE_VADDR};
use crate::memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
use crate::logging::klog;

use crate::scheduler::{yield_to_scheduler,SchedulerCommand};

use super::apic::{send_icr,LocalID};

extern "sysv64" {
    static processors_started: AtomicU16;
    fn ap_trampoline_realmode() -> !;
}

const IPI_INIT: u8 = 0b101u8;
const IPI_SIPI: u8 = 0b110u8;
unsafe fn send_ipi(apic_id: u8, ipi: u8, vector: u8){
    assert!(MULTIPROCESSING_READY.load(Ordering::Relaxed), "Cannot send IPIs before APIC is configured!");
    assert!(ipi <= 0b111u8, "Invalid IPI code!");
    klog!(Debug, PROCESSOR_MANAGEMENT_SMP, "Sending IPI {:b} Vec={:x} to CPU {}", ipi, vector, apic_id);
    let mut ipi_value: u64 = 0;
    ipi_value |= (apic_id as u64) << 56;  // destination
    ipi_value |= (0b01_0_00u64  ) << 11;  // reserved bits + destination mode = physical
    ipi_value |= (ipi as u64    ) << 8;
    ipi_value |=  vector as u64  ;
    send_icr(ipi_value);
}
unsafe fn send_init(apic_id: u8){
    send_ipi(apic_id, IPI_INIT, 0);
}
unsafe fn send_sipi(apic_id: u8, vector: u8){
    send_ipi(apic_id, IPI_SIPI, vector);
}

/* Send an INIT-SIPI-SIPI sequence to a processor, starting it up. This will block until the processor has started.
    This function is not reentrant, as the value it watches does not differentiate between processors.
    Thus, processors must be started one-by-one. */
pub unsafe fn init_processor(apic_id: u8){
    klog!(Info, PROCESSOR_MANAGEMENT_SMP, "Starting AP CPU {}", apic_id);
    send_init(apic_id);
    // Wait a short while for the processor to wake up
    yield_to_scheduler(SchedulerCommand::SleepOneTick);
    
    let current_processors_started: u16 = processors_started.load(Ordering::SeqCst);
    let target: u8 = ((ap_trampoline_realmode as usize) / 4096).try_into().expect("SIPI Target out-of-bounds");
    // Send up to 3 SIPIs (usually takes 2 or sometimes 1) to see if/when the processor starts executing code
    let mut i = 0;
    while processors_started.load(Ordering::Relaxed) < current_processors_started+1 {
        i += 1;
        if i > 3 {
            klog!(Warning, PROCESSOR_MANAGEMENT_SMP, "Failed to wake AP CPU {}", apic_id);
            return;
        }
        
        send_sipi(apic_id, target);
        yield_to_scheduler(SchedulerCommand::SleepOneTick);
    }
}

static MULTIPROCESSING_READY: AtomicBool = AtomicBool::new(false);
/* Get the system ready for multiprocessing. */
pub fn init_multiprocessing(){
    klog!(Debug, PROCESSOR_MANAGEMENT_SMP, "Initialising multicore support...");
    // Get APIC ID or something??
    // idk what to do with it but ok
    // Maybe store it somewhere? Set up cpu-locals or something and put it there?
    let apic_id: u8 = LocalID::read_apic_id();
    
    // Signal that multiprocessing support is ready
    MULTIPROCESSING_READY.store(true, Ordering::Release);
    // Log message
    klog!(Info, PROCESSOR_MANAGEMENT_SMP, "Multicore support enabled. Bootstrap Processor ID = {} (APIC ID {})", get_cpu_id(), apic_id);
}

/* Get the ID of the current CPU. */
#[inline(always)]
pub fn get_cpu_id() -> usize {
    if !MULTIPROCESSING_READY.load(Ordering::Relaxed) { return 0xFFFF; }  // until we're multiprocessing ready, we can't know our own CPU ID
    LocalID::read_apic_id() as usize
}