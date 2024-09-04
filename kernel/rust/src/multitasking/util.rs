
pub type TaskEntryPoint = extern "sysv64" fn() -> !;
pub type TaskEntryPointV<T> = extern "sysv64" fn(*mut T) -> !;

/// Create and start a new kernel task on the current CPU, with the default stack size and settings
/// Returns the task ID.
pub fn spawn_kernel_task(entry: TaskEntryPoint) -> usize {
    let kstack = crate::memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
    let task = super::Task::new_kernel_task(entry, alloc::boxed::Box::new(kstack));
    let task_id = task.task_id();
    super::scheduler::push_task(task);
    task_id
}
pub fn spawn_kernel_task_v<T:Sized>(entry: TaskEntryPointV<T>, arg: *mut T) -> usize {
    let kstack = crate::memory::alloc_util::AllocatedStack::allocate_ktask().unwrap();
    let task = super::Task::new_kernel_task_v(entry, alloc::boxed::Box::new(kstack), arg);
    let task_id = task.task_id();
    super::scheduler::push_task(task);
    task_id
}

macro_rules! def_task_fn {
    {$vis:vis task fn $name:ident($($arg:ident : $argty:ty),*) $body:block} => {
        $vis mod $name {
            use super::*;
            use $crate::multitasking::scheduler::terminate_current_task;
            use alloc::boxed::Box;
            pub struct Args { $($arg : $argty),* }
            pub fn inner($($arg : $argty),*) $body
            
            /// SAFETY: ptr must be a pointer obtained by Box::into_raw(Args), and is cleaned up/freed by this function
            pub extern "sysv64" fn entry(ptr:*mut Args) -> ! {
                {
                    let Args{$($arg),*} = Box::into_inner(unsafe{Box::from_raw(ptr)});
                    inner($($arg),*);
                }
                terminate_current_task();
            }
            pub fn spawn($($arg : $argty),*) -> usize {
                let args = Box::new(Args{$($arg),*});
                $crate::multitasking::util::spawn_kernel_task_v(entry, Box::into_raw(args))
            }
        }
    };
}
pub(crate) use def_task_fn;

def_task_fn! {
    pub task fn call_task_dyn(closure:Box<dyn FnOnce()>){
        // Box<dyn> is a fat pointer, so it doesn't fit in a single register
        // Thus, we box it here and then box it again inside Args{} ^
        
        // We're inside the task
        closure();
    }
}
