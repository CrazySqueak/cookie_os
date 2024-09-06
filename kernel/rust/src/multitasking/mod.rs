pub mod cpulocal;

pub mod fixedcpulocal;
pub use fixedcpulocal::{get_cpu_num};

crate::arch_specific_module!(pub mod arch);