
/// A type is SyscallFFISafe if the type has a defined representation, and if all possible values of the underlying data are valid representations of the type.
/// SyscallFFISafe: Pointers, integers. Syscall FFI unsafe: booleans (only 0 and 1 are defined), floats (for some reason. book says so and they know more than me so I'll take their word on it)
/// In other words, SyscallFFISafe types are "robust" types.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be safely used at a Syscall FFI boundrary",
    label = "`{Self}` is not a robust type",
    note = "If it implements `SyscallFFIMarshallable`, consider wrapping it in an `FFIMarshalled<{Self}>`",
)]
pub unsafe trait SyscallFFISafe: Sized {}
/// A type is SyscallFFIMarshallable if it can be converted to a SyscallFFISafe type, either by reinterpreting it (with a check for validity) or similar.
/// Examples: Enums with a defined integer tag (can be implemented using the decl_ffi_safe! macro), booleans
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be marshalled as it does not implement `SyscallFFIMarshallable`",
    label = "`{Self}` does not implement `SyscallFFIMarshallable`",
)]
pub trait SyscallFFIMarshallable: Sized {
    type As: SyscallFFISafe;
    fn marshall(value: Self) -> Self::As;
    /// Attempt to de-marshall the value, returning None if the value is an invalid representation.
    fn demarshall(value: Self::As) -> Option<Self>;
    
    fn marshalled(self) -> Self::As { Self::marshall(self) }
}
impl<T> SyscallFFIMarshallable for T where T: SyscallFFISafe {
    type As = T;
    #[inline(always)]
    fn marshall(value: Self) -> Self::As { value }
    #[inline(always)]
    fn demarshall(value: Self::As) -> Option<Self> { Some(value) }
}
//impl<T> SyscallFFIMarshallable for T where Option<T>: SyscallFFISafe {
//    type As = Option<T>;
//    fn marshall(value: Self) -> Self::As { Some(value) }
//    fn demarshall(value: Self::As) -> Option<Self> { value }
//}

macro_rules! decl_ffi_safe {
    ($($t:ty),+) => {
        $(unsafe impl SyscallFFISafe for $t{})+
    }
}
decl_ffi_safe!(u8,u16,u32,u64,u128,usize);
decl_ffi_safe!(i8,i16,i32,i64,i128,isize);

// Safety: These are safe to accept, as they cannot be dereferenced except in unsafe code (where one should check they are not dangling and are correctly aligned)
unsafe impl<T> SyscallFFISafe for *const T where T: SyscallFFISafe + Sized {}
unsafe impl<T> SyscallFFISafe for *mut T where T: SyscallFFISafe + Sized {}
// Same goes for optional pointers, thanks to null-pointer optimization
unsafe impl<T> SyscallFFISafe for Option<core::ptr::NonNull<T>> where T: SyscallFFISafe + Sized {}
impl<T> SyscallFFIMarshallable for core::ptr::NonNull<T> where T: SyscallFFISafe + Sized {
    type As = Option<core::ptr::NonNull<T>>;
    fn marshall(value: Self) -> Self::As { Some(value) }
    fn demarshall(value: Self::As) -> Option<Self> { value }
}
// Note: core::ptr::NonNull is not robust unless wrapped in an Optional<>, as NULL is an invalid value for a naked NonNull (but becomes None if interpreted as an Option<NonNull<>>)

impl SyscallFFIMarshallable for bool {
    type As = u8;
    fn marshall(value: Self) -> Self::As { if value { 1 } else { 0 } }
    fn demarshall(value: Self::As) -> Option<Self> { if value == 0 { Some(false) } else if value == 1 { Some(true) } else { None } }
}
impl SyscallFFIMarshallable for () {
    type As = u8;
    fn marshall(_: Self) -> Self::As { 0 }
    fn demarshall(_: Self::As) -> Option<Self> { Some(()) }
}

pub const fn assert_ffi_safe<T:SyscallFFISafe>(){}
pub const fn assert_ffi_marshallable<T:SyscallFFIMarshallable>(){}
