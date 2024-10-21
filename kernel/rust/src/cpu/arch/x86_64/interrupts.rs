use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use pic8259::ChainedPics;
use alloc::boxed::Box;
use crate::logging::klog;
use super::undefined_interrupt_handler_impl;
use crate::multitasking::cpulocal::CpuLocal;
use crate::sync::POnceLock;
// 0x0X and 0x1X - CPU Exceptions

// 0x20 - APIC Timer
pub const APIC_TIMER_VECTOR: u8 = 0x20;
// 0x21 - Kernel Panic
/// May be emitted by the kernel in the case of an unrecoverable panic, to interrupt the other CPUs
pub const KERNEL_PANIC_VECTOR: u8 = 0x21;
// 0x22 - Page Flush
/// Used to request a TLB flush
pub const PAGE_FLUSH_VECTOR: u8 = 0x22;
// ... available
// 0xEX - Legacy PICs (shouldn't trigger but might)
pub const PIC_1_OFFSET: u8 = 0xE0;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
// 0xFF - Spurious
/// Suprious Interrupts: Emitted by the APIC when an interrupt occurs but disappears before the vector is read
/// Must not send an EOI, and the handler should ideally just ignore these.
pub const SPURIOUS_INTERRUPT_VECTOR: u8 = 0xFF;

// IDT
static _LOCAL_IDT: CpuLocal<POnceLock<&'static InterruptDescriptorTable>,false> = CpuLocal::new();
fn init_idt() {
    let idt = Box::leak(Box::new(InterruptDescriptorTable::new()));

    // Initialise handlers
    init_idt_body(idt);

    // Install and load IDT
    _LOCAL_IDT.set(idt).unwrap();
    _LOCAL_IDT.get().unwrap().load();
}

// PICs
pub static LEGACY_PICS: crate::sync::YMutex<ChainedPics> = crate::sync::YMutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

// APIC
/*fn init_xapic(){
    system_apic::with_local_apic(|apic|{
        let mut lvt = apic.lvt.lock();

        // Set spurious vector
        apic.config.lock().siv.set_spurious_vector(SPURIOUS_INTERRUPT_VECTOR);

        // Enable timer
        lvt.timer.set_vector(APIC_TIMER_VECTOR);
        lvt.timer.set_timer_mode(system_apic::TimerMode::Repeating);
        lvt.timer.set_masked(false);
        // Set timer counter
        apic.timer_counters.lock().set_initial_count(500_000);  // magic value for now TODO: figure out how to pick a proper value rather than just eyeballing it
    });
}*/

pub fn init(){
    // Load IDT
    init_idt();
    
    // Map and then disable legacy PIC chips
    unsafe {
        let mut pics = LEGACY_PICS.lock();
        pics.initialize(); pics.disable();
    }

    // TODO: Initialise Local APIC and IOAPIC
    
    // Enable interrupts
    x86_64::instructions::interrupts::enable();
}

pub fn init_ap(){
    // Load IDT
    init_idt();
    
    // Initialise Local APIC
    // TODO
    
    // Enable interrupts
    x86_64::instructions::interrupts::enable();
}

// INTERRUPT HANDLERS
macro_rules! define_interrupts_inner {
    (@idt, $idt:ident, named $vname:ident, $handler:ident) => {
        $idt.$vname.set_handler_fn($handler)
    };
    (@idt, $idt:ident, vector $vname:ident, $handler:ident) => {
        $idt[$vname].set_handler_fn($handler)
    };
    (@wrapbody, exception, $body:block) => {
        // Nothing special to do
        $body
    };
    (@wrapbody, apic, $body:block) => {
        $body

        // send EOI
        // TODO system_apic::with_local_apic(|apic|apic.eoi.signal_eoi());
    };
}
macro_rules! define_interrupts {
    {
        init=fn $initfn:ident;
        handle_undefined=|$udvector:ident,$udsf:ident| $udbody:block;

        $(
            interrupt($ity:ident $vmode:ident $vname:ident)
                fn $fname:ident($stack_frame:ident : $sftype:ty $(, $error_code:ident : $ecty:ty)?)
                    $( -> $rt:ty )? $fbody:block
        )*
    } => {
        pub(super) fn undefined_interrupt_handler($udvector: u8, $udsf: InterruptStackFrame) {
            $udbody
        }
        // initialiser
        fn $initfn(idt: &mut InterruptDescriptorTable) {
            // set all to undefined first
            {
                undefined_interrupt_handler_impl::init_undefined(idt)
            }

            // set each defined item
            $(
                define_interrupts_inner!(@idt, idt, $vmode $vname, $fname);
            )*
        }

        // handlers
        $(
            #[no_mangle]
            extern "x86-interrupt" fn $fname( $stack_frame:$sftype $(,$error_code:$ecty)? ) $( -> $rt )? {
                define_interrupts_inner!(@wrapbody, $ity, $fbody)
            }
        )*
    }
}
define_interrupts! { init=fn init_idt_body;
    handle_undefined=|vector,_stack_frame|{
        klog!(Warning, INTERRUPTS, "Got unexpected interrupt (no handler defined): V={:02x}", vector)
    };

    interrupt(exception named general_protection_fault) fn gp_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> () {
        panic!("General Protection Fault!\n{:?}", stack_frame);
    }
    interrupt(exception named double_fault) fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
        panic!("Double Fault!\n{:?}", stack_frame);  // TODO: setup interrupt handler stacks
    }
    interrupt(exception named page_fault) fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode){
        use x86_64::registers::control::Cr2;
        let accessed_addr = Cr2::read();

        // Page faults are always an error at this point in time
        panic!("Page Fault! Frame={:?} Code={:?} Addr={:?}", stack_frame, error_code, accessed_addr);
    }

    interrupt(apic vector KERNEL_PANIC_VECTOR) fn kernel_panic_interrupt(_stack_frame: InterruptStackFrame){
        panic!("Received kernel panic from another CPU");
    }
    interrupt(apic vector PAGE_FLUSH_VECTOR) fn page_flush_requested(_stack_frame:InterruptStackFrame) {
        todo!() // crate::memory::paging::tlb::perform_pending_flushes
    }

    interrupt(exception vector SPURIOUS_INTERRUPT_VECTOR) fn spurious_interrupt_handler(_stack_frame: InterruptStackFrame) {
        // Do nothing. spurious interrupts are not our problem
        klog!(Info, INTERRUPTS, "Got spurious interrupt.");
    }
}

// APICs
// macro_rules! local_apic_interrupt_handler {
//     ($vector:expr, $name:ident, $sfname:ident, $body:block) => {
//         #[no_mangle]
//         extern "x86-interrupt" fn $name ($sfname: InterruptStackFrame) {
//             $body
//
//             system_apic::with_local_apic(|apic|apic.eoi.signal_eoi());
//         }
//     }
// }
// local_apic_interrupt_handler!(APIC_TIMER_VECTOR, timer_handler, _stackframe, {
//     crate::multitasking::scheduler::_scheduler_tick();
// });

