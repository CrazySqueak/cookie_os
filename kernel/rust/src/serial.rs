
//use uart_16550::{SerialPort,MmioSerialPort};
use lazy_static::lazy_static;

// arch-specific
// TODO: re-organise that mess
mod serial_impl;

//pub trait UARTSerialPortT: core::fmt::Write {
//    fn send(&mut self, data: u8);
//    fn receive(&mut self) -> u8;
//}
//impl UARTSerialPortT for SerialPort {
//    fn send(&mut self, data: u8){self.send(data)}
//    fn receive(&mut self) -> u8 {self.receive() }
//}
//impl UARTSerialPortT for MmioSerialPort {
//    fn send(&mut self, data: u8){self.send(data)}
//    fn receive(&mut self) -> u8 {self.receive() }
//}

// This writer uses spinlocks and without_interrupts(...) to ensure that no deadlocks or race conditions occur
use crate::util::mutex_no_interrupts;
mutex_no_interrupts!(LockedSerialPort, serial_impl::SerialPortType);
impl LockedSerialPort {
    pub fn send(&self, data: u8){
        self.with_lock(|mut w|w.send(data));
    }
    pub fn send_raw(&self, data: u8){
        self.with_lock(|mut w|w.send_raw(data));
    }
    pub fn receive(&self) -> u8 {
        self.with_lock(|mut w|w.receive())
    }
}

lazy_static! {
    pub static ref SERIAL1: LockedSerialPort = LockedSerialPort::wraps(serial_impl::init_serial_1());
}