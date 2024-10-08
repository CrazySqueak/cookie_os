use alloc::format;

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use pic8259::ChainedPics;

//use crate::coredrivers::keyboard_ps2 as keyboard;
use crate::coredrivers::display_vga::VGA_WRITER;

use crate::coredrivers::system_apic;

use alloc::boxed::Box;
use crate::sync::cpulocal::CpuLocalLockedOption;
use crate::sync::kspin::{KRwLockRaw,KMutexRaw};

// 0x0X and 0x1X - CPU Exceptions

// 0x20 - APIC Timer
pub const APIC_TIMER_VECTOR: u8 = 0x20;
// 0x21 - Kernel Panic
/// May be emitted by the kernel in the case of an unrecoverable panic, to interrupt the other CPUs
pub const KERNEL_PANIC_VECTOR: u8 = 0x21;
// ... available
// 0xEX - Legacy PICs (shouldn't trigger but might)
pub const PIC_1_OFFSET: u8 = 0xE0;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
// 0xFF - Spurious
/// Suprious Interrupts: Emitted by the APIC when an interrupt occurs but disappears before the vector is read
/// Must not send an EOI, and the handler should ideally just ignore these.
pub const SPURIOUS_INTERRUPT_VECTOR: u8 = 0xFF;

static _LOCAL_IDT: CpuLocalLockedOption<&'static InterruptDescriptorTable,KMutexRaw,KRwLockRaw> = CpuLocalLockedOption::new();
fn init_idt() {
    let idt = Box::leak(Box::new(InterruptDescriptorTable::new()));
        
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.general_protection_fault.set_handler_fn(gp_fault_handler);
    
    unsafe {
        idt.double_fault.set_handler_fn(double_fault_handler).set_stack_index(super::gdt::DOUBLE_FAULT_IST_INDEX);
    }
    
    // Timer
    idt[APIC_TIMER_VECTOR].set_handler_fn(timer_handler);
    
    // Handle spurious interrupts
    idt[SPURIOUS_INTERRUPT_VECTOR].set_handler_fn(spurious_interrupt_handler);
    // Handle panics
    idt[KERNEL_PANIC_VECTOR].set_handler_fn(kernel_panic_interrupt);
    
    // // Timer
    // idt[PICInterrupt::Timer.as_u8()].set_handler_fn(timer_handler);
    // // PS/2 Keyboard
    // idt[PICInterrupt::Keyboard.as_u8()].set_handler_fn(ps2keyboard_handler);
    // keyboard::set_key_callback(print_key);
    
    // Install and load IDT
    crate::multitasking::without_interruptions(||_LOCAL_IDT.insert_and(idt, |idt|idt.load()));
}

fn init_xapic(){
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
}

pub fn init(){
    // Load IDT
    init_idt();
    
    // Map and then disable legacy PIC chips
    unsafe {
        let mut pics = LEGACY_PICS.lock();
        pics.initialize(); pics.disable();
    }
    
    // Initialise Local APIC
    init_xapic();
    // TODO: Initialise IOAPIC
    
    // Enable interrupts
    x86_64::instructions::interrupts::enable();
}

pub fn init_ap(){
    // Load IDT
    init_idt();
    
    // Initialise xAPIC
    init_xapic();
    
    // Enable interrupts
    x86_64::instructions::interrupts::enable();
}

#[no_mangle]
extern "x86-interrupt" fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode){
    use x86_64::registers::control::Cr2;
    let accessed_addr = Cr2::read();
    
    // Page faults are always an error at this point in time
    panic!("Page Fault! Frame={:?} Code={:?} Addr={:?}", stack_frame, error_code, accessed_addr);
}

#[no_mangle]
extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
    panic!("Double Fault!\n{:?}", stack_frame);
}
#[no_mangle]
extern "x86-interrupt" fn gp_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> () {
    panic!("General Protection Fault!\n{:?}", stack_frame);
}

// Special
#[no_mangle]
extern "x86-interrupt" fn kernel_panic_interrupt(_stack_frame: InterruptStackFrame){
    panic!("Received kernel panic from another CPU");
}

// APICs
macro_rules! local_apic_interrupt_handler {
    ($vector:expr, $name:ident, $sfname:ident, $body:block) => {
        #[no_mangle]
        extern "x86-interrupt" fn $name ($sfname: InterruptStackFrame) {
            $body
            
            system_apic::with_local_apic(|apic|apic.eoi.signal_eoi());
        }
    }
}
local_apic_interrupt_handler!(APIC_TIMER_VECTOR, timer_handler, _stackframe, {
    crate::multitasking::scheduler::_scheduler_tick();
});

#[no_mangle]
extern "x86-interrupt" fn spurious_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Do nothing. spurious interrupts are not our problem
    // should probably add a log message or something but eh
}

// PICs
pub static LEGACY_PICS: crate::sync::YMutex<ChainedPics> = crate::sync::YMutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });
//
//#[repr(u8)]
//pub enum PICInterrupt {
//    Timer = PIC_1_OFFSET+0,
//    Keyboard = PIC_1_OFFSET+1,
//}
//impl PICInterrupt {
//    fn as_u8(self) -> u8 { self as u8 }
//}
//
//macro_rules! pic_interrupt_handler {
//    ($vector:expr, $name:ident, $body:block) => {
//        #[allow(unused_variables)]  // shut the fuck up so i can actually see the important errors/warnings
//        extern "x86-interrupt" fn $name (stack_frame: InterruptStackFrame) {
//            $body
//            
//            unsafe {
//                PICS.lock()
//                    .notify_end_of_interrupt($vector);
//            }
//        }
//    }
//}
//pic_interrupt_handler!(PICInterrupt::Timer.as_u8(), timer_handler, {
//    crate::multitasking::scheduler::_scheduler_tick();
//});
//
//// PS/2 Keyboard
//pic_interrupt_handler!(PICInterrupt::Keyboard.as_u8(), ps2keyboard_handler, {
//    // Safety: This is the interrupt handler that gets called when another byte is ready
//    // this should never be called otherwise
//    unsafe { keyboard::KEYBOARD.recv_next_byte(); };
//});
//fn print_key(key: pc_keyboard::DecodedKey){
//    // Echo on-screen
//    match key {
//        pc_keyboard::DecodedKey::Unicode(chr) => VGA_WRITER.write_string(&format!("{}",chr)),
//        _ => {},
//    }
//}