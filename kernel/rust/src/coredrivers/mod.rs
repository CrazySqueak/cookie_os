/* Core drivers are statically linked modules, linked with the kernel,
so that they are accessible at boot time.
Unlike regular drivers, they do not execute as processes, nor load from disk, as they are intended for use during boot time
(prior to initialising the process table or having access to the filesystem)

By the time the OS has finished booting, these should have been replaced with normal drivers that operate normally.
*/

#[cfg_attr(target_arch = "x86_64", path = "keyboard/ps2_x86_64.rs")]
pub mod keyboard_ps2;
#[cfg_attr(target_arch = "x86_64", path = "serial/uart_x86_64.rs")]
pub mod serial_uart;