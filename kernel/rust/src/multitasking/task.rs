use super::scheduler::StackPointer;

static NEXT_ID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// The type of task (i.e. where it came from / what it's for)
#[derive(Debug)]
pub enum TaskType {
    /// An anonymous kernel task
    KernelTask,
}
pub struct Task {
    pub(super) task_id: usize,
    pub(super) task_type: TaskType,
    
    pub(super) rsp: usize,
}
impl Task {
    pub unsafe fn new_with_rsp(task_type: TaskType, rsp: StackPointer) -> Self {
        Self {
            task_id: NEXT_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            task_type,
            rsp: rsp as usize
        }
    }
    
    pub fn task_id(&self) -> usize { self.task_id }
    pub fn task_type(&self) -> &TaskType {
        &self.task_type
    }
    
    #[inline]
    pub(super) fn set_rsp(&mut self, rsp: StackPointer){
        self.rsp = rsp as usize
    }
    #[inline]
    pub fn get_rsp(&self) -> StackPointer {
        self.rsp as StackPointer
    }
}