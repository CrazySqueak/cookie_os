use uart_16550::SerialPort;

pub fn init_serial_1() -> SerialPort {
    let mut serial_port = unsafe { SerialPort::new(0x3F8) };
    serial_port.init();
    serial_port
}

pub type SerialPortType = SerialPort;