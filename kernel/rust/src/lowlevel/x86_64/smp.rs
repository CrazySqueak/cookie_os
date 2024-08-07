use core::sync::atomic::{AtomicU16,Ordering,AtomicBool};
use crate::sync::{SchedulerYield,Mutex}; use spin::RelaxStrategy;
use x86_64::registers::model_specific::Msr;
use core::ptr::write_volatile;
use crate::memory::paging::global_pages::{KERNEL_PTABLE,KERNEL_PTABLE_VADDR};
use crate::memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
use crate::logging::klog;

extern "sysv64" {
    static processors_started: AtomicU16;
    fn ap_trampoline_realmode() -> !;
}

pub const APIC_MAPPED_ADDR: usize = 0xFEE00_000;

static ICR_LO_ADDR: usize = 0xFEE0_0300 + KERNEL_PTABLE_VADDR;
static ICR_HI_ADDR: usize = 0xFEE0_0310 + KERNEL_PTABLE_VADDR;
const IPI_INIT: u8 = 0b101u8;
const IPI_SIPI: u8 = 0b110u8;
fn send_ipi(apic_id: u8, ipi: u8, vector: u8){
    assert!(MULTIPROCESSING_READY.load(Ordering::Relaxed), "Cannot send IPIs before APIC is configured!");
    assert!(ipi <= 0b111u8, "Invalid IPI code!");
    klog!(Debug, PROCESSOR_MANAGEMENT_SMP, "Sending IPI {:b} Vec={:x} to CPU {}", ipi, vector, apic_id);
    let mut ipi_value: u64 = 0;
    ipi_value |= (apic_id as u64) << 56;  // destination
    ipi_value |= (0b01_0_00u64  ) << 11;  // reserved bits + destination mode = physical
    ipi_value |= (ipi as u64    ) << 8;
    ipi_value |=  vector as u64  ;
    
    let icr_l = ICR_LO_ADDR as *mut u32;
    let icr_h = ICR_HI_ADDR as *mut u32;
    unsafe {
        // send IPI
        write_volatile(icr_h, ((ipi_value&0xFFFFFFFF_00000000)>>32) as u32);
        write_volatile(icr_l, ( ipi_value&0x00000000_FFFFFFFF     ) as u32);
        // wait for IPI to send
        SchedulerYield::relax();
        while (*icr_l)&0b01_0000_0000_0000 != 0 { SchedulerYield::relax() };
    }
}
fn send_init(apic_id: u8){
    send_ipi(apic_id, IPI_INIT, 0);
}
fn send_sipi(apic_id: u8, vector: u8){
    send_ipi(apic_id, IPI_SIPI, vector);
}

/* Send an INIT-SIPI-SIPI sequence to a processor, starting it up. This will block until the processor has started.
    This function is not reentrant, as the value it watches does not differentiate between processors.
    Thus, processors must be started one-by-one. */
pub unsafe fn init_processor(apic_id: u8){
    klog!(Info, PROCESSOR_MANAGEMENT_SMP, "Starting AP CPU {}", apic_id);
    send_init(apic_id);
    
    let current_processors_started: u16 = processors_started.load(Ordering::SeqCst);
    let target: u8 = ((ap_trampoline_realmode as usize) / 4096).try_into().expect("SIPI Target out-of-bounds");
    // TODO: do this properly
    send_sipi(apic_id, target);
    send_sipi(apic_id, target);
    while processors_started.load(Ordering::Relaxed) < current_processors_started+1 { SchedulerYield::relax() }
}

static SVR_ADDR: usize = 0xffff8000_FEE00_0F0;
static APIC_ID_ADDR: usize = 0xffff8000_FEE00_020;

static MULTIPROCESSING_READY: AtomicBool = AtomicBool::new(false);
/* Get the system ready for multiprocessing. */
pub fn init_multiprocessing(){
    klog!(Debug, PROCESSOR_MANAGEMENT_SMP, "Initialising multicore support...");
    // Map page
    let apic_addr = APIC_MAPPED_ADDR;
    let apic_buf = KERNEL_PTABLE.allocate_at(apic_addr + KERNEL_PTABLE_VADDR, 0x1000).expect("Unable to map APIC MMIO!");
    apic_buf.set_base_addr(apic_addr, PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::PINNED));
    apic_buf.leak();  // leak so that it doesn't get deallocated
    
    // Enable APIC in SVR
    let svr_ptr = SVR_ADDR as *mut u32;
    unsafe {
        let mut svr = *svr_ptr;
        svr |= 0b01_0000_0000;  // enable APIC
        svr |= 0x69;  // set spurious interrupt to 0x69 so i can tell if that's what's causing all the double faults
        write_volatile(svr_ptr, svr);
    }
    // Get APIC ID or something??
    // idk what to do with it but ok
    // Maybe store it somewhere? Set up cpu-locals or something and put it there?
    let apic_id_ptr = APIC_ID_ADDR as *const u32;
    let apic_id: u8 = ((unsafe { *apic_id_ptr } & 0xFF000000)>>24).try_into().unwrap();
    
    // Signal that multiprocessing support is ready
    MULTIPROCESSING_READY.store(true, Ordering::Release);
    // Log message
    klog!(Info, PROCESSOR_MANAGEMENT_SMP, "Multicore support enabled. Bootstrap Processor ID = {} (APIC ID {})", get_cpu_id(), apic_id);
}

/* Get the ID of the current CPU. */
#[inline(always)]
pub fn get_cpu_id() -> usize {
    if !MULTIPROCESSING_READY.load(Ordering::Relaxed) { return 0xFFFF; }  // until we're multiprocessing ready, we can't know our own CPU ID
    ((unsafe { *(APIC_ID_ADDR as *const u32) } & 0xFF000000)>>24) as usize
}