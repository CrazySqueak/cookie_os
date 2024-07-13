use alloc::format;
use core::{file,line};

#[derive(Debug,Clone,Copy,PartialOrd,Ord,PartialEq,Eq)]
#[repr(u8)]
pub enum LogLevel {
    // Debug: very specific, low-level information.
    Debug = 0,
    // Info: General information, status messages, etc.
    Info = 1,
    // Warning: Abnormal conditions that may require attention
    Warning = 2,
    // Severe: Abnormal conditions that may directly impair the operation of the system
    Severe = 3,
    // Critical: Abnormal conditions that will cause issues - generally require things to shut down soon
    Critical = 4,
    // Fatal: Things can no longer continue in this state. Usually followed by a kernel panic
    Fatal = 5,
}
impl LogLevel {
    pub fn name(self) -> &'static str {
        use LogLevel::*;
        match self {
            Debug    => "DBG",
            Info     => "INFO",
            Warning  => "WARN",
            Severe   => "SEVERE",
            Critical => "CRITICAL",
            Fatal    => "FATAL ERROR",
        }
    }
}

use crate::coredrivers::serial_uart::SERIAL1;
use crate::util::LockedWrite;
pub fn _kernel_log(level: LogLevel, component: &str, msg: &str){
    let msg = format!("{}: [{}] {} - {}\n", 0, level.name(), component, msg);
    
    let _ = SERIAL1.write_str(&msg);
}

macro_rules! klog {
    ($level: ident, $component: literal, $template:expr, $($x:expr),*) => {
        crate::logging::klog!($level, $component, &alloc::format!($template, $($x),*));
    };
    
    ($level: ident, $component:literal, $msg: expr) => {
        {
            use crate::logging::LogLevel::*;
            if $level >= crate::logging::configured_log_level($component.as_bytes()) {
                crate::logging::_kernel_log($level, &alloc::format!("{}@{}:{}", $component, file!(), line!()), $msg);
            }
        }
    };
}
pub(in crate) use klog;

// Logging config
// Returns the minimum log level for the chosen component
// log messages below that are ignored
// Note: more specific configs (e.g. x.y.z) override less specific ones (e.g. x.y)
pub const fn configured_log_level(component: &[u8]) -> LogLevel {
    use LogLevel::*;
    match component {
        
        //b"memory.physical" => Info,
        b"memory.physical.buddies" => Warning,
        b"memory.physical.memmap" => Warning,
        
        b"memory.kheap" => Debug,
        
        b"default" => Info,
        _ => {
            // Split string by "." if possible
            let mut dot_pos: Option<usize> = None;
            let mut i = component.len()-1; while i > 0 { if component[i] == b'.' { dot_pos = Some(i); break; }; i -= 1; }
            if let Some(dot_pos) = dot_pos {
                let (prefix, _) = component.split_at(dot_pos);
                configured_log_level(prefix)
            }
            else { configured_log_level(b"default") }
        }
    }
}