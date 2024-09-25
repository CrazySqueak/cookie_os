
pub mod cpulocal;

pub mod waitlist;
pub use waitlist::WaitingList;

pub mod spin;
// Note: kspin has special rules for its usage to avoid deadlocks. See kspin's documentation for details.
pub mod kspin;
mod yspin;
pub use yspin::*;
mod wlock;
pub use wlock::*;

mod promise;
pub use promise::*;
mod queue;
pub use queue::*;