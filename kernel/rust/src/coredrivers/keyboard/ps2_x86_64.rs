use alloc::collections::VecDeque;
use alloc::vec::Vec;
use alloc::vec;

use lazy_static::lazy_static;
use x86_64::instructions::port::Port;
use spin::{Mutex,MutexGuard};
use pc_keyboard::{DecodedKey,Keyboard,ScancodeSet1};

const KB_IO_PORT: u16 = 0x60;

#[derive(Debug,Clone,Copy)]
pub enum KeyboardCommand {
    SetLEDs(u8),  // TODO add more
}
impl KeyboardCommand {
    pub fn as_bytes(&self) -> Vec<u8>{
        match self {
            KeyboardCommand::SetLEDs(o1) => vec![0xED, *o1],
        }
    }
}

pub enum KeyboardState {
    // Ready to send a command
    Ready,
    // Awaiting an acknowlegement
    AwaitingAcknowledgement(KeyboardCommand),
    // Awaiting a multibyte scancode
    AwaitingScanCode,
}
// Only the consumer is in charge of PS2KeyboardState
// whereas the producers may need queue access
pub struct PS2KeyboardState{
    port: Port<u8>,
    
    status: KeyboardState,
    
    key_callback: fn(DecodedKey),
    key_decoder: Keyboard<pc_keyboard::layouts::Us104Key, ScancodeSet1>,
}

pub struct PS2Keyboard {
    state: Mutex<PS2KeyboardState>,
    command_queue: Mutex<VecDeque<KeyboardCommand>>,
}
impl PS2Keyboard {
    fn new(key_callback: fn(DecodedKey)) -> Self {
        let me = Self {
            state: Mutex::new(PS2KeyboardState {
                port: Port::new(KB_IO_PORT),
                status: KeyboardState::Ready,
                
                key_callback: key_callback,
                key_decoder: Keyboard::new(ScancodeSet1::new(), pc_keyboard::layouts::Us104Key, pc_keyboard::HandleControl::Ignore),
            }),
            command_queue: Mutex::new(VecDeque::new()),
        };
        
        me
    }
    
    // UNSAFE: Calling this when a new byte is not waiting may cause issues. THIS SHOULD ONLY BE CALLED FROM THE PS/2 KEYBOARD INTERRUPT HANDLER.
    /** Recieve the next byte from the controller. */
    pub unsafe fn recv_next_byte(&self){
        let mut state = self.state.lock();
        let command_queue = self.command_queue.lock();
        let byte = state.port.read();
        
        match state.status {
            KeyboardState::Ready | KeyboardState::AwaitingScanCode => {
                // scan code
                let decode_result = state.key_decoder.add_byte(byte);
                if let Ok(Some(key_event)) = decode_result {
                    if let Some(key) = state.key_decoder.process_keyevent(key_event) {
                        (state.key_callback)(key);
                    }
                    // set state back to ready
                    state.status = KeyboardState::Ready;
                } else if let Ok(None) = decode_result {
                    // Multi-part key - set state to "awaiting"
                    state.status = KeyboardState::AwaitingScanCode;
                }
                // Otherwise idk
            }
            
            KeyboardState::AwaitingAcknowledgement(cmd) => {
                match byte {
                    0xFA | 0xEE => {  // ACK or ECHO
                        // Set next state
                        match state.status {
                            _ => state.status = KeyboardState::Ready,  // Nothing else keeping us
                        }
                        
                        // And send the next command if there is one
                        self.send_next_command_if_ready(&mut state, command_queue);
                    }
                    
                    0xFE => {  // RESEND
                        self.send_command(&mut state, cmd);
                    }
                    
                    0xAA | 0xFC | 0xFD => { // Self-Test passed/failed
                        // we ignore these for the time being
                    }
                    
                    _ => todo!(),
                }
            }
        }
    }
    
    /** Send a command. */
    fn send_command(&self, state: &mut MutexGuard<PS2KeyboardState>, command: KeyboardCommand){
        for b in command.as_bytes() { unsafe { state.port.write(b); } };
        
        state.status = KeyboardState::AwaitingAcknowledgement(command);
    }
    fn send_next_command_if_ready(&self, state: &mut MutexGuard<PS2KeyboardState>, mut command_queue: MutexGuard<VecDeque<KeyboardCommand>>){
        if let KeyboardState::Ready = state.status {
            if let Some(cmd) = command_queue.pop_front(){
                self.send_command(state, cmd);
            }
        }
    }
    
    /** Add a command to the queue. */
    pub fn queue_command(&self, cmd: KeyboardCommand){
        // this without_interrupts is necessary
        // as if an interrupt triggered recv_byte during this
        // it would cause a deadlock
        // (the only two in-points that lock the state are this and recv_byte (which fires on interrupt and before we acknowledge it) so we should be fine)
        crate::lowlevel::without_interrupts(|| {
            let state_opt = self.state.try_lock();  // if nothing else is keeping us, we lock the state so we can send the command immediately
            
            let mut command_queue = self.command_queue.lock();
            command_queue.push_back(cmd);
            
            // Send command immediately if we're ready
            if let Some(mut state) = state_opt {
                self.send_next_command_if_ready(&mut state, command_queue);
            }
        });
    }
}

// Public API
lazy_static! {
    pub static ref KEYBOARD: PS2Keyboard = PS2Keyboard::new(handle_key);
}
static KEY_HANDLER: Mutex<fn(DecodedKey)> = Mutex::new(ignore_key);
fn ignore_key(_k: DecodedKey){}
fn handle_key(k: DecodedKey){
    KEY_HANDLER.lock()(k)
}
pub fn set_key_callback(f: fn(DecodedKey)){
    crate::lowlevel::without_interrupts(|| {  // must be without interrupts to avoid a deadlock if the keyboard interrupt fires while we do this
        (*KEY_HANDLER.lock()) = f;
    });
}