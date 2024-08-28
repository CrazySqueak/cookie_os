pub trait LockedNoInterrupts {
    type Wraps;
    fn with_lock<R,F: FnOnce(crate::sync::MutexGuard<Self::Wraps>)->R>(&self, f: F) -> R;
}

#[macro_export]
macro_rules! mutex_no_interrupts {
    ($name:ident, $($lifes:lifetime),*, $wraps:ty) => {
        use $crate::LockedNoInterrupts;
        #[repr(transparent)]
        pub struct $name<$($lifes),*> {
            pub(crate) inner: crate::sync::Mutex<$wraps>
        }
        impl<$($lifes),*> $name<$($lifes),*>{
            pub const fn wraps(inner: $wraps) -> Self {
                Self {
                    inner: crate::sync::Mutex::new(inner)
                }
            }
        }
        impl<$($lifes),*> LockedNoInterrupts for $name<$($lifes),*>{
            type Wraps = $wraps;
            fn with_lock<R,F: FnOnce(crate::sync::MutexGuard<Self::Wraps>)->R>(&self, f: F) -> R{
                crate::lowlevel::without_interrupts(||f(self.inner.lock()))
            }
        }
    };
    ($name:ident, $wraps:ty) => {
        // an empty lifetime parameter has to be passed due to the comma there being required o.o
        // this pattern helps out in case only one comma is passed
        mutex_no_interrupts!($name,,$wraps);
    };
}
pub use mutex_no_interrupts;

use core::fmt::Write;
pub trait LockedWrite {
    fn write_str(&self, s: &str) -> Result<(), core::fmt::Error>;
    fn write_char(&self, c: char) -> Result<(), core::fmt::Error>;
    fn write_fmt(&self, args: core::fmt::Arguments<'_>) -> Result<(), core::fmt::Error>;
}
impl<T:LockedNoInterrupts> LockedWrite for T
    where T::Wraps : core::fmt::Write 
{  // idk how and/or if this works but ok
    fn write_str(&self, s: &str) -> Result<(), core::fmt::Error>{
        self.with_lock(|mut w|w.write_str(s))
    }
    fn write_char(&self, c: char) -> Result<(), core::fmt::Error>{
        self.with_lock(|mut w|w.write_char(c))
    }
    fn write_fmt(&self, args: core::fmt::Arguments<'_>) -> Result<(), core::fmt::Error>{
        self.with_lock(|mut w|w.write_fmt(args))
    }
}