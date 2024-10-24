//! Contains the syscall definitions themselves.
//!
//! These definitions are as they're called by the syscall handler,
//! and as such do not include the wrapping handled by invoke()/etc.
//!
//! For example, a syscall that returns a value of type `u32` will actually return
//! a value of type `Result<u32,SyscallInvokeError>`, as the syscall itself may not even
//! be known to the kernel, or supported.

use crate::syscore::syscalls::define_syscall_interface;
// 
// define_syscall_interface! {
//     tag = pub enum(u32) SyscallTag;
//     handler_table = pub struct SyscallHandlerTable;
//     handler_types = pub mod handlers;
//     invokers = pub mod invokers;
//     num_syscalls = pub const NUM_SYSCALLS;
// 
//     /// Get the highest known syscall tags used by the kernel's version of the library.
//     ///
//     /// This is not an indication of compatibility, but simply states the kernel's highest known
//     /// syscall number.
//     ///
//     /// Attempting to call syscalls with tags > this number is guaranteed to result in an error.
//     extern syscall(0x0000) fn GetMaxKnownSyscall() -> u32;
// }
