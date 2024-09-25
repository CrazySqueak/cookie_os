
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
    (@return_type, ) => { () };
    (@return_type, $rt:ty) => { $rt };
    
    {$vis:vis task fn $name:ident($($arg:ident : $argty:ty),*) $(-> $rt:ty)? $body:block} => {
        $vis mod $name {
            use super::*;
            use $crate::multitasking::scheduler::terminate_current_task;
            use $crate::multitasking::util::def_task_fn;
            use $crate::sync::promise::{PromiseFulfiller,Promise};
            use alloc::boxed::Box;
            pub struct Args { $($arg : $argty,)* __out: PromiseFulfiller<def_task_fn!(@return_type, $($rt)?)> }
            pub fn inner($($arg : $argty,)*) $(-> $rt)? $body
            
            /// SAFETY: ptr must be a pointer obtained by Box::into_raw(Args), and is cleaned up/freed by this function
            pub extern "sysv64" fn entry(ptr:*mut Args) -> ! {
                {
                    let Args{$($arg,)* __out } = Box::into_inner(unsafe{Box::from_raw(ptr)});
                    let result = inner($($arg,)*);
                    let _ = __out.complete(result);
                }
                terminate_current_task();
            }
            /// Spawn the task as a new Kernel Task, returning the task ID and a promise for the return value (or a Promise<()> which is fulfilled upon task completion, if no return value is given)
            pub fn spawn($($arg : $argty,)*) -> (usize, Promise<def_task_fn!(@return_type, $($rt)?)>) {
                let (__out_tx, __out_rx) = Promise::<def_task_fn!(@return_type, $($rt)?)>::new();
                let args = Box::new(Args{$($arg,)* __out: __out_tx});
                let task_id = $crate::multitasking::util::spawn_kernel_task_v(entry, Box::into_raw(args));
                (task_id, __out_rx)
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
