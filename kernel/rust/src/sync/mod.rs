pub mod baselocks;
pub mod spinlocks;
pub mod nointerruptionslocks;

// == MUTEXES AND RWLOCKS ==
/// llspin - Low-level spin locks (used for implementing no_interruptions and cpulocals)
pub mod llspin;
// kspin - Kernel Spin (similar to llspin but wraps the guard with a no_interruptions guard to disable interrupts and yielding while held)
pub mod kspin;
/// yspin - Yield Spin (yields to scheduler, task only)
pub mod yspin;
pub use yspin::*;
// hspin - Hybrid Spin (yields to scheduler if possible, otherwise behaves like kspin. Always applies no_interruptions, even if used inside a task)
pub mod hspin;
// wlock - Waiting List Locks (ticket mutexes using WaitingLists)
// TODO

// == OTHER USEFUL PRIMITIVES ==
/// waitlist - "Waiting Lists" for tasks to queue up on
pub mod waitlist;
pub use waitlist::*;
/// queue - A locked queue
pub mod queue;
pub use queue::*;
/// promise - Completable promises
pub mod promise;
pub use promise::*;