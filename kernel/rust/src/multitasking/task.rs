use super::scheduler::StackPointer;

use crate::memory::alloc_util::AnyAllocatedStack;
use alloc::boxed::Box;

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
    pub(super) stack_allocation: Option<Box<dyn AnyAllocatedStack>>,
}
impl Task {
    pub unsafe fn new_with_rsp(task_type: TaskType, rsp: StackPointer, stack_allocation: Option<Box<dyn AnyAllocatedStack>>) -> Self {
        Self {
            task_id: NEXT_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            task_type,
            rsp: rsp as usize,
            stack_allocation: stack_allocation,
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