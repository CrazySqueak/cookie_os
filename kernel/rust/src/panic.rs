use crate::multitasking;


/// This function is called on panic.
/// If the task that triggers a panic is not a bootstrap or scheduler task, then the task terminates and a log message is printed. (this is still a serious issue, however, as a kernel-level panic at the wrong time could lock up the system - kernel panics should represent errors that present immediate threat to the operating system e.g. where locking up the whole system would be a viable handling strategy)
/// Otherwise, a kernel panic occurs and the whole thing goes tits-up
use crate::logging::{klog,emergency_kernel_log};
use core::panic::PanicInfo;
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
        todo!()//lowlevel::halt();
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
        
        // // Begin shutting down CPUs - requires MMIO to be mapped for APIC to work - med risk but high importance. paging/MMIO is mapped way before multitasking is configured anyway
        // unsafe{lowlevel::emit_panic();}
        
        // Print more debug information - requires scheduler to be in a sane state (either there or not there) - med risk
        let context = multitasking::ExecutionContext::current();
        emergency_kernel_log!("Execution Context: {}\r\n", context);
        
        // // Forcefully acquire a reference to the current writer, bypassing the lock (which may have been locked at the time of the panic and will not unlock as we don't have stack unwinding)
        // // Requires MMIO to be mapped - med risk
        // let mut writer = unsafe{let wm=&*VGA_WRITER;wm.force_unlock();wm.lock()};
        // writer.set_colour(display_vga::VGAColour::new(display_vga::BaseColour::LightGray,display_vga::BaseColour::Red,true,false));
        // // Write message and location to screen
        // let _ = write!(writer, "\n\nKERNEL PANICKED (@{}): {}", context, _info);
        
        // Attempt to perform backtrace
        // TODO
        
        // Finally, halt
        emergency_kernel_log!("\r\nEnd of kernel panic. Halting.");
        todo!()//lowlevel::halt();
    }
}
