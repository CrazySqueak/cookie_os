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
pub fn _kernel_log(level: LogLevel, component: &str, msg: &str, context: crate::multitasking::ExecutionContext, file: &str, line: u32, column: u32){
    let msg = format!("[{}] {}: {} - {} ({}:{}:{})\r\n", level.name(), context, component, msg, file, line, column);
    
    let _ = SERIAL1.write_str(&msg);
}

#[macro_export]
macro_rules! klog {
    ($level: ident, $component:ident, $template:literal, $($x:expr),*) => {
        crate::logging::klog!($level, $component, &alloc::format!($template, $($x),*))
    };
    
    ($level: ident, $component:ident, $msg: expr) => {
        {
            use $crate::LogLevel::*;
            use $crate::contexts::*;
            use crate::multitasking::ExecutionContext;
            if const { ($level as u8) >= ($component as u8) } { $crate::_kernel_log($level, stringify!($component), $msg, ExecutionContext::current(), file!(), line!(), column!()) };
        }
    };
}
pub use klog;

// For use in emergency situations, such as a kernel panic.
// uses no heap allocation and forcibly bypasses locks
// generally if you're using this function, shit is fucked and the program should be due to abort any second now
#[macro_export]
macro_rules! emergency_kernel_log {
    ($($msg:tt)*) => {
        crate::lowlevel::without_interrupts(||{
            use crate::coredrivers::serial_uart::SERIAL1;
            use core::fmt::Write;
            let mut serial = unsafe { loop { match SERIAL1.inner.try_lock() {
                    Some(lock) => break lock,
                    None => SERIAL1.inner.force_unlock(),
                }
            }};
            let _ = write!(serial, $($msg)*);
        })
    }
}
pub use emergency_kernel_log;

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
    def_context!(BOOT, ROOT);  // boot-time top-level progress messages
    def_context!(MEMORY, ROOT);
      def_context!(MEMORY_PAGING, MEMORY);
        def_context!(MEMORY_PAGING_CONTEXT, MEMORY_PAGING);
        def_context!(MEMORY_PAGING_GLOBALPAGES, MEMORY_PAGING);
        def_context!(MEMORY_PAGING_ALLOCATOR, MEMORY_PAGING);
          def_context!(MEMORY_PAGING_ALLOCATOR_MLFF, MEMORY_PAGING_ALLOCATOR);
        def_context!(MEMORY_PAGING_MAPPINGS, MEMORY_PAGING);
        def_context!(MEMORY_PAGING_TLB, MEMORY_PAGING, Info);
      def_context!(MEMORY_KHEAP, MEMORY);
      def_context!(MEMORY_PHYSICAL, MEMORY);
        def_context!(MEMORY_PHYSICAL_BUDDIES, MEMORY_PHYSICAL, Warning);
        def_context!(MEMORY_PHYSICAL_RAMMAP, MEMORY_PHYSICAL);
        def_context!(MEMORY_PHYSICAL_ALLOCATOR, MEMORY_PHYSICAL, Warning);
      def_context!(MEMORY_ALLOCUTIL, MEMORY);
    def_context!(FEATURE_FLAGS, ROOT, Debug);
    def_context!(SCHEDULER, ROOT);
    def_context!(CPU_MANAGEMENT, ROOT);
    def_context!(COREDRIVERS, ROOT);
      def_context!(COREDRIVERS_XAPIC, COREDRIVERS);
      def_context!(COREDRIVERS_VGA, COREDRIVERS);
}
