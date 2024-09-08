use super::scheduler::StackPointer;

// (placeholder)
trait AnyAllocatedStack { fn bottom_vaddr(&self) -> usize; } //use crate::memory::alloc_util::AnyAllocatedStack;
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
    /// Create a new task using the given stack and entry point. This calls _cs_new to initialise the stack with the necessary function pointer, and then returns a suitable task.
    pub fn new_kernel_task(entry_point: extern "sysv64" fn() -> !, stack: Box<dyn AnyAllocatedStack>) -> Task {
        // Initialise stack and get RSP
        // SAFETY: This MUST have exclusive access to the given stack, which is enforced (hopefully) by ownership rules
        // (stack must grow downwards. TODO: Kill myself if I'm ever porting this to an architecture where the stack grows upwards)
        unsafe {
            let rsp = super::arch::context_switch::_cs_new(entry_point, stack.bottom_vaddr() as *const u8);
            Self::new_with_rsp(TaskType::KernelTask, rsp, Some(stack))
        }
    }
    pub fn new_kernel_task_v<T:Sized>(entry_point: extern "sysv64" fn(*mut T) -> !, stack: Box<dyn AnyAllocatedStack>, arg1: *mut T) -> Task {
        unsafe {
            // SAFETY: It's fine to cast *mut T to *mut u8 as we've already checked that the arg1 pointer and the argument in the fn(...) are the same type
            let rsp = super::arch::context_switch::_cs_newv(core::mem::transmute(entry_point), stack.bottom_vaddr() as *const u8, arg1 as *mut u8);
            Self::new_with_rsp(TaskType::KernelTask, rsp, Some(stack))
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
