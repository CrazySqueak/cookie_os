pub mod spin;
pub use self::spin::{Mutex,MutexGuard,RwLock,RwLockReadGuard,RwLockWriteGuard,RwLockUpgradableGuard,SchedulerYield,AlwaysPanic};

pub mod cpulocal;