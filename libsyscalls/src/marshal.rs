use crate::safety::{SyscallFFISafe,SyscallFFIMarshallable};

// == ENUMS ==
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

//  == STRUCTS ==
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

//  == TUPLES ==
// Tuples are serialized as an equivalent ordered marshallable struct
// (note: only implemented for tuples up to arity 12)
macro_rules! impl_tuple_marshal {
    ($typename:ident, $($tparam:ident),+) => {
        #[allow(dead_code)]
        #[allow(non_snake_case)]
        #[repr(C)]
        pub struct $typename<$($tparam: $crate::safety::SyscallFFIMarshallable),+> {
            $($tparam: $tparam::As),+
        }
        // SAFETY: As $typename contains the marshalled version of each type (::As), which is required to be safe, this struct is safe as well
        unsafe impl<$($tparam: $crate::safety::SyscallFFIMarshallable),+> $crate::safety::SyscallFFISafe for $typename<$($tparam),+> {}
        
        impl<$($tparam: $crate::safety::SyscallFFIMarshallable),+> $crate::safety::SyscallFFIMarshallable for ($($tparam,)+) {
            type As = $typename<$($tparam),+>;
            fn marshall(value: Self) -> Self::As {
                #[allow(non_snake_case)]
                let ($($tparam,)+) = value;
                $typename {$(
                    $tparam: $crate::safety::SyscallFFIMarshallable::marshall($tparam),
                )+}
            }
            fn demarshall(value: Self::As) -> Option<Self> {
                #[allow(non_snake_case)]
                let $typename { $($tparam,)+ } = value;
                Some(($(
                    $crate::safety::SyscallFFIMarshallable::demarshall($tparam)?,
                )+))
            }
        }
    }
}
impl_tuple_marshal!(FFI1Tuple, A);
impl_tuple_marshal!(FFI2Tuple, A, B);
impl_tuple_marshal!(FFI3Tuple, A, B, C);
impl_tuple_marshal!(FFI4Tuple, A, B, C, D);
impl_tuple_marshal!(FFI5Tuple, A, B, C, D, E);
impl_tuple_marshal!(FFI6Tuple, A, B, C, D, E, F);
impl_tuple_marshal!(FFI7Tuple, A, B, C, D, E, F, G);
impl_tuple_marshal!(FFI8Tuple, A, B, C, D, E, F, G, H);
impl_tuple_marshal!(FFI9Tuple, A, B, C, D, E, F, G, H, I);
impl_tuple_marshal!(FFI10Tuple, A, B, C, D, E, F, G, H, I, J);
impl_tuple_marshal!(FFI11Tuple, A, B, C, D, E, F, G, H, I, J, K);
impl_tuple_marshal!(FFI12Tuple, A, B, C, D, E, F, G, H, I, J, K, L);

const _: () = {
    crate::safety::assert_ffi_marshallable::<(u32,u32)>();
    crate::safety::assert_ffi_marshallable::<(u32,bool)>();
    crate::safety::assert_ffi_marshallable::<(u32,(u32,u32,bool))>();
};

//  == UTIL TYPE ==
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