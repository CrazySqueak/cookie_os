use spin::relax::RelaxStrategy;
use super::baselocks::{MutexStrategy,RwLockStrategy};

pub struct SpinLockStrategy<R:RelaxStrategy>(core::marker::PhantomData<R>);
impl<R:RelaxStrategy> MutexStrategy for SpinLockStrategy<R> {
    const INIT: Self = Self(core::marker::PhantomData);
    #[inline(always)]
    fn on_unlock(&self){}
    #[inline(always)]
    fn lock_relax(&self){R::relax()}
}
impl<R:RelaxStrategy> RwLockStrategy for SpinLockStrategy<R> {
    const INIT: Self = Self(core::marker::PhantomData);
    
    #[inline(always)]
    fn on_read_unlock(&self, new_reader_count: usize){}
    #[inline(always)]
    fn read_relax(&self){R::relax()}
    
    #[inline(always)]
    fn on_write_unlock(&self){}
    #[inline(always)]
    fn write_relax(&self){R::relax()}
    
    #[inline(always)]
    fn on_upgradeable_release(&self){}
    #[inline(always)]
    fn upgradeable_relax(&self){R::relax()}
    #[inline(always)]
    fn upgrade_u2w_relax(&self){R::relax()}
    
    #[inline(always)]
    fn on_downgrade_w2r(&self){}
    #[inline(always)]
    fn on_downgrade_w2u(&self){}
    #[inline(always)]
    fn on_downgrade_u2r(&self){}
}
