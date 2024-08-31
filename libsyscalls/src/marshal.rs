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

/// Internal macro used for several different operations
macro_rules! marshal_operations {
    // @is_safe: Algorithm to determine whether a struct contains only FFI-safe types, or at least one FFI-marshall type
    (@is_safe, marshall$(,)? $($imode:ident),* -> $callback_name:ident@$callback_mode:ident($($args:tt)*)) => { $crate::marshal::$callback_name!(@$callback_mode, marshall, $($args)*); };
    (@is_safe, safe, $($imode:ident),* -> $callback_name:ident@$callback_mode:ident($($args:tt)*)) => { $crate::marshal::marshal_operations!(@is_safe, $($imode),* -> $callback_name@$callback_mode($($args)*)); };
    (@is_safe, safe -> $callback_name:ident@$callback_mode:ident($($args:tt)*)) => { $crate::marshal::$callback_name!(@$callback_mode, safe, $($args)*); };
    
    // @assert_safety: Asserts that a given item is what it's said to be
    (@assert_safety, $itype:ty, safe) => { const _:() = $crate::safety::assert_ffi_safe::<$itype>(); };
    (@assert_safety, $itype:ty, marshall) => { const _:() = $crate::safety::assert_ffi_marshallable::<$itype>(); };
    
    // @marshall_value: Convert a marshallable value in a variable of the given name to its marshalled form, storing it in a variable of the same name
    (@marshall_value, $name:ident, safe, $itype:ty) => {};
    (@marshall_value, $name:ident, marshall, $itype:ty) => {
        let $name = $crate::marshal::FFIMarshalled::from($name);
    };
    // @demarshall_value: (and vice versa, returning if None is returned)
    (@demarshall_value, $name:ident, safe, $itype:ty) => {};
    (@demarshall_value, $name:ident, marshall, $itype:ty) => {
        let $name = $crate::marshal::FFIMarshalled::try_into($name)?;
    };
    // @marshall_type: Convert a marshallable type to its marshalled version
    (@marshall_type, safe, $itype:ty) => { $itype };
    (@marshall_type, marshalled, $itype:ty) => { $crate::marshal::FFIMarshalled<$itype> };
    
}
pub(crate) use marshal_operations;

/// Define a struct that is Syscall FFI-compatible.
macro_rules! ffi_struct {
    
    // @impl_ffi: Implement ffi-safe or FFI-marshall. Usually used as a callback with is_safe
    (@impl_ffi, safe, $name:ident, $($iname:ident $imode:ident $itype:ty),+ $(as $mname:ident)?) => {
        // SAFETY: As the struct is repr(C) and all items are robust, the struct itself can be considered robust
        #[automatically_derived]
        unsafe impl $crate::safety::SyscallFFISafe for $name {}
    };
    (@impl_ffi, marshall, $name:ident, $($iname:ident $imode:ident $itype:ty),+ as $mname:ident) => {
        #[automatically_derived]
        impl $crate::safety::SyscallFFIMarshallable for $name {
            type As = $mname;
            
            fn marshall(value: Self) -> Self::As {
                $(
                    let $iname = value.$iname;
                    $crate::marshal::marshal_operations!(@marshall_value, $iname, $imode, $itype);
                )+
                $mname { $($iname:$iname),+ }
            }
            fn demarshall(value: Self::As) -> Option<Self> {
                $(
                    let $iname = value.$iname;
                    $crate::marshal::marshal_operations!(@demarshall_value, $iname, $imode, $itype);
                )+
                Some(Self { $($iname:$iname),+ })
            }
        }
    };
    
    // @mstruct: Construct the marshalled version of a marshallable struct, by munching each item in term and converting it to the correct mode
    {@mstruct : $($tokens:tt)*} => {};
    {@mstruct $mname:ident: $($attrs:meta)* ; $vis:vis , $($($repr:ident),+)? ; $($($pattrs:meta)* $pvis:vis $pname:ident safe $ptype:ty),* ; $($cattrs:meta)* $cvis:vis $cname:ident safe $ctype:ty $(, $($iattrs:meta)* $ivis:vis $iname:ident $imode:ident $itype:ty)* } => {
        ffi_struct! {@mstruct $mname: $($attrs)* ; $vis , $($($repr),+)? ; $($($pattrs)* $pvis $pname safe $ptype,)* $($cattrs:meta)* $cvis $cname safe $ctype ; $($($iattrs)* $ivis $iname $imode $itype),* }
    };
    {@mstruct $mname:ident: $($attrs:meta)* ; $vis:vis , $($($repr:ident),+)? ; $($($pattrs:meta)* $pvis:vis $pname:ident safe $ptype:ty),* ; $($cattrs:meta)* $cvis:vis $cname:ident marshall $ctype:ty $(, $($iattrs:meta)* $ivis:vis $iname:ident $imode:ident $itype:ty)* } => {
        ffi_struct! {@mstruct $mname: $($attrs)* ; $vis , $($($repr),+)? ; $($($pattrs)* $pvis $pname safe $ptype,)* $($cattrs:meta)* $cvis $cname safe $crate::marshal::FFIMarshalled<$ctype> ; $($($iattrs)* $ivis $iname $imode $itype),* }
    };
    {@mstruct $mname:ident: $($attrs:meta)* ; $vis:vis , $($($repr:ident),+)? ; $($($pattrs:meta)* $pvis:vis $pname:ident safe $ptype:ty),* ; } => {
        ffi_struct! {
            $(#[$attrs])*
            $vis extern$(($($repr),+))? struct $mname {
                $(
                    $(#[$pattrs])*
                    $pvis $pname: safe $ptype,
                )*
            }
        }
    };
    
    {
        $(#[$attrs:meta])*
        $vis:vis extern$(($($repr:ident),+))? struct $name:ident $(marshalled as $mname:ident)? {
            $(
                $(#[$iattrs:meta])*
                $ivis:vis $iname:ident: $imode:ident $itype:ty
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
        // Implement either safe or marshall, depending on whether this has marshalled items
        $crate::marshal::marshal_operations!(@is_safe, $($imode),+ -> ffi_struct@impl_ffi($name, $($iname $imode $itype),+ $(as $mname)?));
        
        // Assert that each type is what it's said to be
        $($crate::marshal::marshal_operations!(@assert_safety, $itype, $imode);)+
        
        // Implement the marshalled version
        ffi_struct! { @mstruct $($mname)?: $($attrs)* ; $vis , $($($repr),+)? ; ; $($($iattrs)* $ivis $iname $imode $itype),+ }
    }
}
pub(crate) use ffi_struct;
ffi_struct! {
    #[allow(dead_code)]
    pub(crate) extern struct A {
        x: safe u32,
        y: safe i64,
        z: safe u16,
    }
}
ffi_struct! {
    #[allow(dead_code)]
    pub(crate) extern(packed) struct B marshalled as BFFI {
        x: safe u32,
        y: safe i64,
        z: marshall bool,
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