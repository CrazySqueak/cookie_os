use core::sync::atomic::{AtomicU16,Ordering};
use crate::sync::{SchedulerYield,Mutex}; use spin::RelaxStrategy;
use x86_64::registers::model_specific::Msr;

extern "sysv64" {
    static processors_started: AtomicU16;
    fn ap_trampoline_realmode() -> !;
}

static MSR_ICR: Mutex<Msr> = Mutex::new(Msr::new(0x830));
const IPI_INIT: u8 = 0b101u8;
const IPI_SIPI: u8 = 0b110u8;
fn send_ipi(apic_id: u8, ipi: u8, vector: u8){
    assert!(ipi <= 0b111u8);
    let mut ipi_value: u64 = 0;
    ipi_value |= (apic_id as u64) << 56;  // destination
    ipi_value |= (0b01_0_00u64  ) << 11;  // reserved bits + destination mode = physical
    ipi_value |= (ipi as u64    ) << 8;
    ipi_value |=  vector as u64  ;
    
    let mut icr = MSR_ICR.lock();
    unsafe {
        // send IPI
        icr.write(ipi_value);
        // wait for IPI to send
        while icr.read()&0b01_0000_0000_0000 != 0 { SchedulerYield::relax() };
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
    send_init(apic_id);
    
    let current_processors_started: u16 = processors_started.load(Ordering::Relaxed);
    let target: u8 = ((ap_trampoline_realmode as usize) / 4096).try_into().expect("SIPI Target out-of-bounds");
    // TODO: do this properly
    send_sipi(apic_id, target);
    send_sipi(apic_id, target);
    while processors_started.load(Ordering::Relaxed) < current_processors_started+1 { SchedulerYield::relax() }
}

/* Get the system ready for multiprocessing. */
pub fn init_multiprocessing(){
    // Enable APIC SVR or something idk
    let svr_paddr = 0x0FEE000F0usize;
    let svr_vaddr = svr_paddr + 0xffff8000_00000000;
    let svr_ptr = svr_vaddr as *mut u32;
    unsafe {
        let svr = *svr_ptr;
        core::ptr::write_volatile(svr_ptr, svr | 0b01_0000_0000);
    }
}