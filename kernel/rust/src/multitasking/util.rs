
pub type TaskEntryPoint = extern "sysv64" fn() -> !;

/// Create and start a new kernel task on the current CPU, with the default stack size and settings
/// Returns the task ID.
pub fn spawn_kernel_task(entry: TaskEntryPoint) -> usize {
    let kstack = crate::memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
    let task = super::Task::new_kernel_task(entry, alloc::boxed::Box::new(kstack));
    let task_id = task.task_id();
    super::scheduler::push_task(task);
    task_id
}