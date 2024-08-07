pub mod spin;
pub use self::spin::{Mutex,MutexGuard,RwLock,RwLockReadGuard,RwLockWriteGuard,RwLockUpgradableGuard,SchedulerYield,AlwaysPanic};

// TODO: Decide where this goes
use alloc::collections::BTreeMap; use core::default::Default; use alloc::sync::Arc;
use crate::lowlevel::without_interrupts;
use crate::scheduler::get_cpu_id;
pub struct CPULocal<T: Default>(RwLock<BTreeMap<usize,Arc<T>>,::spin::relax::Spin>);  // note: we cannot yield with interrupts disabled unless we want to break the system
impl<T: Default> CPULocal<T> {
    pub const fn new() -> Self {
        Self(RwLock::new(BTreeMap::new()))
    }
    
    pub fn get(&self) -> Arc<T> {
        without_interrupts(||{
            let cpu_id = get_cpu_id();
            crate::logging::klog!(Info,ROOT,"CLGET={}",cpu_id);
            let lock = self.0.read();
            let item = lock.get(&cpu_id);
            if let Some(arc) = item { arc.clone() }
            else {
                drop(lock); let mut lock = self.0.write();
                lock.insert(cpu_id, Arc::new(T::default()));
                lock.get(&cpu_id).unwrap().clone()
            }
        })
    }
}