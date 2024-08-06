use super::context_switch::StackPointer;

/// The type of task (i.e. where it came from / what it's for)
pub enum TaskType {
    /// An anonymous kernel task
    KernelTask,
}
pub struct Task {
    task_type: TaskType,
    
    rsp: usize,
}
impl Task {
    #[inline]
    pub(super) fn set_rsp(&mut self, rsp: StackPointer){
        self.rsp = rsp as usize
    }
    #[inline]
    pub fn get_rsp(&self) -> StackPointer {
        self.rsp as StackPointer
    }
}