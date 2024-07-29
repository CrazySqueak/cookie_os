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
    ($level: ident, $component:ident, $template:expr, $($x:expr),*) => {
        crate::logging::klog!($level, $component, &alloc::format!($template, $($x),*));
    };
    
    ($level: ident, $component:ident, $msg: expr) => {
        {
            use crate::logging::LogLevel::*;
            use crate::logging::contexts::*;
            if const { ($level as u8) >= ($component as u8) } { crate::logging::_kernel_log($level, stringify!($component), $msg) };
        }
    };
}
pub(in crate) use klog;

// Logging contexts allow filtered log levels to be configured per-context
pub mod contexts {
    use super::LogLevel; use LogLevel::*;
    macro_rules! def_context {
        ($id: ident, $parent: ident, $filter_level: ident) => {
            pub const $id: LogLevel = $filter_level;
        };
        ($id: ident, $parent: ident) => {
            pub const $id: LogLevel = $parent;
        };
    }
    
    pub const DEFAULT_MIN_LOG_LEVEL: super::LogLevel = Info;
    pub const ROOT: LogLevel = DEFAULT_MIN_LOG_LEVEL;
    
    // Configure contexts in here! :)
    def_context!(MEMORY, ROOT);
      def_context!(MEMORY_PAGING, MEMORY, Debug);
        def_context!(MEMORY_PAGING_CONTEXT, MEMORY_PAGING);
        def_context!(MEMORY_PAGING_ALLOCATOR, MEMORY_PAGING);
          def_context!(MEMORY_PAGING_ALLOCATOR_MLFF, MEMORY_PAGING_ALLOCATOR);
        def_context!(MEMORY_PAGING_MAPPINGS, MEMORY_PAGING);
        def_context!(MEMORY_PAGING_TLB, MEMORY_PAGING);
      def_context!(MEMORY_KHEAP, MEMORY, Debug);
      def_context!(MEMORY_PHYSICAL, MEMORY);
        def_context!(MEMORY_PHYSICAL_BUDDIES, MEMORY_PHYSICAL, Warning);
        def_context!(MEMORY_PHYSICAL_RAMMAP, MEMORY_PHYSICAL);
        def_context!(MEMORY_PHYSICAL_ALLOCATOR, MEMORY_PHYSICAL);
}

// Returns the minimum log level for the chosen component
// log messages below that are ignored
// Note: more specific configs (e.g. x.y.z) override less specific ones (e.g. x.y)
pub const fn configured_log_level(component: &[u8]) -> LogLevel {
    use LogLevel::*;
    match component {
        b"memory.paging" => Debug,
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