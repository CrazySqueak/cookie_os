pub mod spin;
pub use self::spin::{Mutex,MutexGuard,RwLock,RwLockReadGuard,RwLockWriteGuard,RwLockUpgradableGuard,SchedulerYield};

pub mod cpulocal;
