pub mod spin;
pub use self::spin::{Mutex,MutexGuard,RwLock,RwLockReadGuard,RwLockWriteGuard,RwLockUpgradableGuard,SchedulerYield};

pub mod cpulocal;
pub use cpulocal::{CpuLocal,CpuLocalLockedItem,CpuLocalRWLockedItem,CpuLocalNoInterruptsLockedItem,CpuLocalLockedOption};

