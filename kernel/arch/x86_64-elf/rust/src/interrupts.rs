use core::fmt::Write;

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::serial::SERIAL1;

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(fuck_you_too);
        idt.general_protection_fault.set_handler_fn(fuck_you_2);
        idt
    };
}

pub fn init(){
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame){
    let _ = write!(SERIAL1.lock(),"Breakpoint! Frame={:?}", stack_frame);
    // TODO
}

extern "x86-interrupt" fn fuck_you_too(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode){
    let _ = write!(SERIAL1.lock(),"Page Fault! Frame={:?} Code={:?}", stack_frame, error_code);
}

extern "x86-interrupt" fn fuck_you_2(stack_frame: InterruptStackFrame, error_code: u64){
    let _ = write!(SERIAL1.lock(),"GP Fault! Frame={:?} Code={:?}", stack_frame, error_code);
}