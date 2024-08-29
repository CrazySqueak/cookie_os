//! The 'forward' module contains forward-declarations for many functions defined in later kernel modules, but
//! which are still used in earlier crates.
//! 
//! A good example are the functions for inquiring the scheduler's status, or performing a yield as part of a spinlock.
//! The kernel spinlocks are defined as part of kcore::sync, but they ideally yield to the scheduler if applicable (a fact which cannot easily be worked around).
//!
//! These functions may always be assumed to be safe (unless the forward declaration is for an unsafe function).

pub mod scheduler;
