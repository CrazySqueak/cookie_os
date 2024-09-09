pub mod cpulocal;

pub mod fixedcpulocal;
pub use fixedcpulocal::{get_cpu_num};

crate::arch_specific_module!(pub mod arch);

pub mod interruptions;
pub use interruptions::disable_interruptions;

pub mod scheduler;
pub use scheduler::{is_executing_task,yield_to_scheduler,SchedulerCommand};
pub mod task;
//pub mod util;