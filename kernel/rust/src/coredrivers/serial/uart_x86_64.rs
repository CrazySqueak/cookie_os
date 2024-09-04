
use lazy_static::lazy_static;
use uart_16550::SerialPort;

fn init_serial_1() -> SerialPort {
    let mut serial_port = unsafe { SerialPort::new(0x3F8) };
    serial_port.init();
    serial_port
}
pub type SerialPortType = SerialPort;

// This writer uses spinlocks and without_interrupts(...) to ensure that no deadlocks or race conditions occur
// use crate::util::mutex_no_interrupts;
// mutex_no_interrupts!(LockedSerialPort, SerialPortType);
// impl LockedSerialPort {
//     pub fn send(&self, data: u8){
//         self.with_lock(|mut w|w.send(data));
//     }
//     pub fn send_raw(&self, data: u8){
//         self.with_lock(|mut w|w.send_raw(data));
//     }
//     pub fn receive(&self) -> u8 {
//         self.with_lock(|mut w|w.receive())
//     }
// }
use crate::sync::KMutex;

lazy_static! {
    pub static ref SERIAL1: KMutex<SerialPortType> = KMutex::new(init_serial_1());
}