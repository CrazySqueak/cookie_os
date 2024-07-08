/*#[repr(C)]
struct IDTEntry {
    function_ptr_low  : u16,
    gdt_selector      : u16,
    idt_flags         : IDTFlags,
    function_ptr_mid  : u16,
    function_ptr_high : u32,
    reserved          : u32,
}
impl IDTEntry {
    unsafe fn get_function_ptr(&self) -> fn(){
        let fn_addr = ((self.function_ptr_high as u64)<<32) | ((self.function_ptr_mid as u64)<<16) | (self.function_ptr_low as u64);
        core::mem::transmute::<*const (),fn()>(fn_addr as *const ())
    }
    fn set_function_ptr(&mut self, handler: fn()){
        let fn_addr = (handler as *const ()) as u64;
        self.function_ptr_high = ((fn_addr>>32)&0x0_FFFF_FFFF) as u32;
        self.function_ptr_mid =  ((fn_addr>>16)&0x0_FFFF     ) as u16;
        self.function_ptr_low =  ( fn_addr     &0x0_FFFF     ) as u16;
    }
}

#[repr(transparent)]
struct IDTFlags {
    flags: u16
}
const IDTFLAG_IST_INDEX = 0b0000_0000_0000_0111;
const IDTFLAG_GATE_TYPE = 0b0000_0001_0000_0000;
const IDTFLAG__REQ_ONES = 0b0000_1110_0000_0000;
const IDTFLAG__REQ_ZERO = 0b0001_0000_0000_0000;
const IDTFLAG_______DPL = 0b0110_0000_0000_0000;
const IDTFLAG___PRESENT = 0b1000_0000_0000_0000;
impl IDTFlags {
    pub fn new() -> Self {
        IDTFlags {
            flags: IDTFLAG__REQ_ONES | IDTFLAG___PRESENT
        }
    }
}*/

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
