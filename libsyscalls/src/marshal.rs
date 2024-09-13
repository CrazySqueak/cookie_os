use crate::safety::{SyscallFFISafe,SyscallFFIMarshallable};

/// Define an enum that is Syscall FFI-compatible, with an automatic implementation of SyscallFFIMarshallable.
/// (for convenience this also derives Clone,Copy,Debug as these are all applicable to any int-backed C-style enum)
/// Syntax: [pub] enum(<integer type>) <name> { [variants with explicit discriminants...] }
macro_rules! ffi_enum {
    {
        $(#[$attrs:meta])*
        $vis:vis enum($inttype:ty) $name:ident {
            $(
                $(#[$vattrs:meta])*
                $vname:ident = $vtag:literal
            ),+
            $(,)?
        }
    } => {
        $(#[$attrs])*
        #[derive(Debug,Clone,Copy)]
        #[repr($inttype)]
        $vis enum $name {
            $(
                $(#[$vattrs])*
                $vname = $vtag,
            )+
        }
        #[automatically_derived]
        impl $crate::safety::SyscallFFIMarshallable for $name {
            type As = $inttype;
            fn marshall(value: Self) -> Self::As {
                value as $inttype
            }
            fn demarshall(value: Self::As) -> Option<Self> {
                match value {
                    $($vtag => Some(Self::$vname),)+
                    _ => None,
                }
            }
        }
    }
}
pub(crate) use ffi_enum;
ffi_enum! {
    #[allow(dead_code)]
    pub(crate) enum(u16) Example {
        Test0 = 0,
        Test1 = 1,
        SixtyNine = 69,
        FourTwenty = 420,
    }
}

macro_rules! ffi_struct {
    (@impl_ffi for $name:ident, $vis:vis, $(($($repr:ident),+))?, ; $($ivis:vis $iname:ident: $itype:ty),+) => {
        // SAFETY: As the struct is repr(C) and all items are robust, the struct itself can be considered robust
        #[automatically_derived]
        unsafe impl $crate::safety::SyscallFFISafe for $name {}
        // Ensure all items are FFI-safe
        const _: () = const { $(
            $crate::safety::assert_ffi_safe::<$itype>();
        )+ };
    };
    (@impl_ffi for $name:ident, $vis:vis, $(($($repr:ident),+))?, $mname:ident ; $($ivis:vis $iname:ident: $itype:ty),+) => {
        // Create Marshalled variant
        $crate::marshal::ffi_struct! {
            #[allow(dead_code)]
            $vis extern$(($($repr),+))? struct $mname {
                $( $ivis $iname: <$itype as $crate::safety::SyscallFFIMarshallable>::As ),+
            }
        }
        // Impl SyscallFFIMarshallable
        #[automatically_derived]
        impl $crate::safety::SyscallFFIMarshallable for $name {
            type As = $mname;
            
            fn marshall(value: Self) -> Self::As {
                $(
                    let $iname = $crate::safety::SyscallFFIMarshallable::marshall(value.$iname);  // SyscallFFISafe also implement SyscallFFIMarshallable (even though it's a no-op)
                )+
                $mname { $($iname:$iname),+ }
            }
            fn demarshall(value: Self::As) -> Option<Self> {
                $(
                    let $iname = $crate::safety::SyscallFFIMarshallable::demarshall(value.$iname)?;  // SyscallFFISafe also implement SyscallFFIMarshallable (even though it's a no-op)
                )+
                Some(Self { $($iname:$iname),+ })
            }
        }
        // Assert that all items may be marshalled
        const _: () = const { $(
            $crate::safety::assert_ffi_marshallable::<$itype>();
        )+ };
    };
    
    {
        $(#[$attrs:meta])*
        $vis:vis extern$(($($repr:ident),+))? struct $name:ident $(marshalled as $mname:ident)? {
            $(
                $(#[$iattrs:meta])*
                $ivis:vis $iname:ident: $itype:ty
            ),+
            $(,)?
        }
    } => {
        $(#[$attrs])*
        #[repr(C $(,$($repr),+)?)]
        $vis struct $name {
            $(
                $(#[$iattrs])*
                $ivis $iname: $itype,
            )+
        }
        // Implement either safe or marshall
        $crate::marshal::ffi_struct!(@impl_ffi for $name, $vis, $(($($repr),+))?, $($mname)? ; $($ivis $iname: $itype),+);
    };
}
pub(crate) use ffi_struct;
ffi_struct! {
    #[allow(dead_code)]
    pub(crate) extern struct A {
        x: u32,
        y: i64,
        z: u16,
    }
}
ffi_struct! {
    #[allow(dead_code)]
    pub(crate) extern(packed) struct B marshalled as BFFI {
        x: u32,
        y: i64,
        z: bool,
    }
}

#[repr(transparent)]
pub struct FFIMarshalled<T:SyscallFFIMarshallable>(T::As);
unsafe impl<T:SyscallFFIMarshallable> SyscallFFISafe for FFIMarshalled<T> {}
impl<T:SyscallFFIMarshallable> From<T> for FFIMarshalled<T> {
    fn from(value: T) -> Self {
        Self(SyscallFFIMarshallable::marshall(value))
    }
}
impl<T:SyscallFFIMarshallable> FFIMarshalled<T> {
    pub fn try_into(self) -> Option<T> {
        SyscallFFIMarshallable::demarshall(self.0)
    }
}