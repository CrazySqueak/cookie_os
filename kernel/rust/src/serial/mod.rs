
//use uart_16550::{SerialPort,MmioSerialPort};
use spin::Mutex;
use lazy_static::lazy_static;

// arch-specific
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

lazy_static! {
    pub static ref SERIAL1: Mutex<serial_impl::SerialPortType> = Mutex::new(serial_impl::init_serial_1());
}