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
pub fn _kernel_log(level: LogLevel, feature: &str, msg: &str){
    let msg = format!("{}: [{}] {} - {}\n", 0, level.name(), feature, msg);
    
    let _ = SERIAL1.write_str(&msg);
}

macro_rules! klog {
    ($level: ident, $feature: literal, $template:expr, $($x:expr),*) => {
        crate::logging::klog!($level, $feature, &format!($template, $($x),*));
    };
    
    (Debug, $feature:literal, $msg:expr) => {
        #[cfg(debug_assertions)]
        crate::logging::klog!(_dbghandled, Debug, $feature, $msg);
    };
    ($level: ident, $feature:literal, $msg: expr) => {
        crate::logging::klog!(_dbghandled, $level, $feature, $msg);
    };
    
    (_dbghandled, $level: ident, $feature:literal, $msg: expr) => {
        #[cfg(feature = $feature)]
        {
            use crate::logging::LogLevel::*;
            crate::logging::_kernel_log($level, &format!("{}@{}:{}", $feature, file!(), line!()), $msg);
        }
    };
}
pub(in crate) use klog;