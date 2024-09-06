pub mod cpulocal;

pub mod cpunum;
pub use cpunum::{init_cpu_num,get_cpu_num};

crate::arch_specific_module!(pub mod arch);