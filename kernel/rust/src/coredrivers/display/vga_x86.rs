
use lazy_static::lazy_static;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BaseColour{
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
}

const FLAG_BLINK: u8 = 0b1000_0000;
const FLAG_LIGHT: u8 = 0b0000_1000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VGAColour(u8);
impl VGAColour {
    pub fn new(foreground: BaseColour, background: BaseColour, bright: bool, blink: bool) -> Self {
        let bright_flag = if bright { FLAG_LIGHT } else { 0 };
        let blink_flag = if blink { FLAG_BLINK } else { 0 };
        VGAColour((background as u8)<<4 | (foreground as u8) | bright_flag | blink_flag)
    }
    
    pub fn is_blinking(&self) -> bool {
        (self.0 & FLAG_BLINK) != 0
    }
    pub fn is_bright(&self) -> bool {
        (self.0 & FLAG_LIGHT) != 0
    }
    pub fn foreground(&self) -> BaseColour {
        match self.0 & 0b0111 { // match bottom 3 bits
            0 => BaseColour::Black,
            1 => BaseColour::Blue,
            2 => BaseColour::Green,
            3 => BaseColour::Cyan,
            4 => BaseColour::Red,
            5 => BaseColour::Magenta,
            6 => BaseColour::Brown,
            7 => BaseColour::LightGray,
            _ => unreachable!(),
        }
    }
    pub fn background(&self) -> BaseColour {
        match (self.0>>4) & 0b0111 { // match bg bits
            0 => BaseColour::Black,
            1 => BaseColour::Blue,
            2 => BaseColour::Green,
            3 => BaseColour::Cyan,
            4 => BaseColour::Red,
            5 => BaseColour::Magenta,
            6 => BaseColour::Brown,
            7 => BaseColour::LightGray,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct VGAChar {
    ascii_code: u8,
    colour: VGAColour,
}

use volatile::Volatile;
const VGA_HEIGHT: usize = 25;
const VGA_WIDTH: usize = 80;
#[repr(transparent)]
pub struct VGABuffer {
    chars: [[Volatile<VGAChar>; VGA_WIDTH]; VGA_HEIGHT],
}
impl VGABuffer {
    pub fn put_byte(&mut self, x: usize, y: usize, c: u8, colour: VGAColour) {
        self.chars[y][x].write(VGAChar { ascii_code: c, colour: colour });
    }
    pub fn put_vgachar(&mut self, x: usize, y: usize, chr: VGAChar){
        self.chars[y][x].write(chr);
    }
    
    pub fn get_vgachar(&mut self, x: usize, y: usize) -> VGAChar { self.chars[y][x].read() }
}

// We're mapped to the higher half, so we have to remember to OFFSET the memory-mapped I/O otherwise we get a page fault
// which ends badly because the current page fault handler involves writing to the screen. oops
pub const VGA_BUFFER_PHYSICAL: usize = 0xb8000;
#[cfg(target_arch = "x86_64")]
pub const VGA_BUFFER_ADDR: usize = 0xFFFF800000000000 + VGA_BUFFER_PHYSICAL;

fn get_standard_vga_buffer() -> &'static mut VGABuffer {
    return unsafe { &mut *(VGA_BUFFER_ADDR as *mut VGABuffer) };
}

pub struct VGAConsoleWriter<'a> {
    column_pos: usize,
    row_pos: usize,
    colour: VGAColour,
    buffer: &'a mut VGABuffer,
}
impl<'a> VGAConsoleWriter<'a> {
    pub fn new_with_buffer(buffer: &'a mut VGABuffer) -> Self {
        VGAConsoleWriter {
            column_pos: 0, row_pos: 0,
            colour: VGAColour::new(BaseColour::LightGray, BaseColour::Black, false, false),
            buffer: buffer,
        }
    }
    
    pub fn advance_right(&mut self){
        self.column_pos += 1;
        if self.column_pos >= VGA_WIDTH {
            self.new_line();
        }
    }
    pub fn advance_down(&mut self){
        self.row_pos += 1;
        if self.row_pos >= VGA_HEIGHT {
            self.scroll(1);
        }
    }
    pub fn return_to_left(&mut self){
        self.column_pos = 0;
    }
    
    pub fn new_line(&mut self){
        self.return_to_left();
        self.advance_down();
    }
    
    pub fn set_colour(&mut self, colour: VGAColour){
        self.colour = colour
    }
    
    // Scroll the screen up to make space for new text
    pub fn scroll(&mut self, nlines: usize){
        // Move text up
        for newy in 0..(VGA_HEIGHT-nlines) {
            let oldy = newy+nlines;
            for x in 0..VGA_WIDTH {
                let old = self.buffer.get_vgachar(x, oldy);
                self.buffer.put_vgachar(x, newy, old);
            }
        }
        
        // Clear bottom lines
        for y in (VGA_HEIGHT-nlines)..VGA_HEIGHT {
            for x in 0..VGA_WIDTH {
                self.buffer.put_byte(x,y, b' ', self.colour);
            }
        }
        
        // Move cursor to correct position
        if self.row_pos < nlines { self.row_pos = 0; }
        else { self.row_pos -= nlines; }
    }
    
    pub fn write_byte(&mut self, byte: u8){
        match byte {
            b'\n' => { self.new_line(); }
            byte => {
                self.buffer.put_byte(self.column_pos, self.row_pos, byte, self.colour);
                self.advance_right();
            }
        }
    }
    
    pub fn write_string(&mut self, s: &str){
        for c in s.bytes(){
            self.write_byte(c);
        }
    }
}

// This writer uses spinlocks and without_interrupts(...) to ensure that no deadlocks or race conditions occur
use crate::util::mutex_no_interrupts;
mutex_no_interrupts!(LockedVGAConsoleWriter, 'a, VGAConsoleWriter<'a>);
impl<'a> LockedVGAConsoleWriter<'a>{
    pub fn scroll(&self, nlines: usize){
        self.with_lock(|mut w|w.scroll(nlines));
    }
    
    pub fn write_byte(&self, byte: u8){
        self.with_lock(|mut w|w.write_byte(byte));
    }
    pub fn write_string(&self, s: &str){
        self.with_lock(|mut w|w.write_string(s));
    }
}

lazy_static! {
    pub static ref VGA_WRITER: LockedVGAConsoleWriter<'static> = LockedVGAConsoleWriter::wraps(VGAConsoleWriter::new_with_buffer(get_standard_vga_buffer()));
}
