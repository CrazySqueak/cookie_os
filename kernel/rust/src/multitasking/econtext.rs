use super::scheduler;
use super::get_cpu_num;

// A snapshot of the execution context. Mostly useful for annotating log messages and the like.
pub struct ExecutionContext {
    pub cpu_id: usize,
    pub task_id: Option<usize>,
    pub scheduler_clock_ticks: usize,
}
impl ExecutionContext {
    #[inline]
    pub fn current() -> Self {
        Self {
            cpu_id: get_cpu_num(),
            task_id: scheduler::get_executing_task_id(),
            scheduler_clock_ticks: scheduler::get_scheduler_ticks(),
        }
    }
}
use core::fmt;
impl fmt::Display for ExecutionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}t:", self.scheduler_clock_ticks)?;
        write!(f, " CPU{}", self.cpu_id)?;
        
        if let Some(task) = self.task_id {
            write!(f, " TASK {}", task)?;
        } else {
            write!(f, " SCHED")?;
        }
        
        Ok(())
    }
}