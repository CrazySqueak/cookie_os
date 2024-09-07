pub mod baselocks;
pub mod spinlocks;

/// llspin - Low-level spin locks (used for implementing no_interruptions and cpulocals)
pub mod llspin;
// kspin - Kernel Spin (similar to llspin but wraps the guard with a no_interruptions guard to disable interrupts and yielding while held)

/// yspin - Yield Spin (yields to scheduler, task only)
pub mod yspin;
// wlock - Waiting List Locks (ticket mutexes using WaitingLists)

// hspin - Hybrid Spin (yields to scheduler if possible, otherwise behaves like kspin. Always applies no_interruptions, even if used inside a task)
