use alloc::format;

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use pic8259::ChainedPics;
use spin::Mutex;

use crate::util::LockedWrite;
use crate::coredrivers::serial_uart::SERIAL1;
use crate::coredrivers::keyboard_ps2 as keyboard;
use crate::coredrivers::display_vga::VGA_WRITER;

// TODO: Create public API that is as architecture-independent as possible

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(fuck_you_too);
        
        unsafe {
            idt.double_fault.set_handler_fn(double_fault_handler).set_stack_index(super::gdt::DOUBLE_FAULT_IST_INDEX);
        }
        
        // Timer
        idt[PICInterrupt::Timer.as_u8()].set_handler_fn(timer_handler);
        // PS/2 Keyboard
        idt[PICInterrupt::Keyboard.as_u8()].set_handler_fn(ps2keyboard_handler);
        keyboard::set_key_callback(print_key);
        
        idt
    };
}

pub fn init(){
    // Load IDT
    IDT.load();
    // Initialize 1980s PIC chips
    unsafe { PICS.lock().initialize(); }
    // Enable interrupts
    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame){
    let _ = write!(SERIAL1,"Breakpoint! Frame={:?}", stack_frame);
    // TODO
}

extern "x86-interrupt" fn fuck_you_too(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode){
    use x86_64::registers::control::Cr2;
    let accessed_addr = Cr2::read();
    
    // Page faults are always an error at this point in time
    panic!("Page Fault! Frame={:?} Code={:?} Addr={:?}", stack_frame, error_code, accessed_addr);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
    panic!("Double Fault!\n{:?}", stack_frame);
}

// PICs
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
pub static PICS: Mutex<ChainedPics> = Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

#[repr(u8)]
pub enum PICInterrupt {
    Timer = PIC_1_OFFSET+0,
    Keyboard = PIC_1_OFFSET+1,
}
impl PICInterrupt {
    fn as_u8(self) -> u8 { self as u8 }
}

macro_rules! pic_interrupt_handler {
    ($vector:expr, $name:ident, $body:block) => {
        extern "x86-interrupt" fn $name (stack_frame: InterruptStackFrame) {
            $body
            
            unsafe {
                PICS.lock()
                    .notify_end_of_interrupt($vector);
            }
        }
    }
}
pic_interrupt_handler!(PICInterrupt::Timer.as_u8(), timer_handler, {
    let _ = write!(SERIAL1,"Beep");
    // TODO
});

// PS/2 Keyboard
pic_interrupt_handler!(PICInterrupt::Keyboard.as_u8(), ps2keyboard_handler, {
    // Safety: This is the interrupt handler that gets called when another byte is ready
    // this should never be called otherwise
    unsafe { keyboard::KEYBOARD.recv_next_byte(); };
});
fn print_key(key: pc_keyboard::DecodedKey){
    let _ = write!(SERIAL1,"{:?}", key);
    
    // Echo on-screen
    match key {
        pc_keyboard::DecodedKey::Unicode(chr) => VGA_WRITER.write_string(&format!("{}",chr)),
        _ => {},
    }
}