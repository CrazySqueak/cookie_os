
use crate::lowlevel::smp::init_processor;
use core::sync::atomic::{AtomicU16,AtomicUsize,Ordering};
use crate::memory::alloc_util::AllocatedStack;

use crate::scheduler::{yield_to_scheduler,SchedulerCommand};

pub static PROCESSORS_READY: AtomicU16 = AtomicU16::new(1);
// This pointer is used for processor's kernel stacks as they come online
#[used]
#[no_mangle]
static next_processor_stack: AtomicUsize = AtomicUsize::new(0);

pub unsafe fn start_processor(processor_id: usize){
    // Allocate a kernel bootstrap stack for the processor
    let kstack = AllocatedStack::allocate_kboot().unwrap();
    next_processor_stack.store(kstack.bottom_vaddr(), Ordering::SeqCst);
    core::mem::forget(kstack);
    
    // Wake the processor
    let prev_processors_ready = PROCESSORS_READY.load(Ordering::SeqCst);
    init_processor(processor_id.try_into().unwrap());
    // Wait for it to start
    while PROCESSORS_READY.load(Ordering::Relaxed) < prev_processors_ready+1 { yield_to_scheduler(SchedulerCommand::SleepOneTick); };
    // :)
}