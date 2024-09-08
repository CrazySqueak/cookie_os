pub mod cpulocal;

pub mod fixedcpulocal;
pub use fixedcpulocal::{get_cpu_num};

crate::arch_specific_module!(pub mod arch);

pub mod interruptions;

pub mod scheduler;
pub mod task;
//pub mod util;