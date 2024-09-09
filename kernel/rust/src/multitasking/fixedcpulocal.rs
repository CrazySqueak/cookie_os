//! Fixed CPU-local values.
//! Unlike regular CpuLocals, these cannot be allocated dynamically, but must be statically put here.
//! This makes it useful for values such as the cpu number, and so on.
use super::arch::fixedcpulocal as arch;

pub trait FixedCpuLocalDef {
    type Type;
    fn default() -> Self::Type;
    fn get() -> &'static Self::Type;
}
macro_rules! fixed_cpu_local {
    ($vis:vis fixedcpulocal static $name:ident: $type:ty = $default:expr) => {
        // You can define a struct with the same name as a static, provided the struct is a {} struct or a () struct (not a blank one with just a ;).
        // Lazy-static does this internally
        // Idk why the fuck this is allowed but it's convenient here (and it's abstracted away anyhow)
        #[allow(non_camel_case_types)]
        $vis struct $name {}
        impl $crate::multitasking::fixedcpulocal::FixedCpuLocalDef for $name {
            type Type = $type;
            #[inline(always)]
            fn default() -> Self::Type  {
                $default
            }
            #[inline(always)]
            fn get() -> &'static Self::Type {
                &$crate::multitasking::fixedcpulocal::get_fixed_cpu_locals().$name
            }
        }
        impl core::ops::Deref for $name {
            type Target = $type;
            #[inline(always)]
            fn deref(&self) -> &Self::Target {
                <Self as $crate::multitasking::fixedcpulocal::FixedCpuLocalDef>::get()
            }
        }
        $vis static $name: $name = $name{};
    }
}
pub(crate) use fixed_cpu_local;

macro_rules! def_cl_struct {
    {$vis:vis struct $sn:ident (init fn $fn:ident, temp mod $fclm:ident) { $(use $mp:path as $i:ident),* $(,)? } } => {
        mod $fclm {
            #![allow(non_snake_case)]
            use $crate::multitasking::fixedcpulocal::FixedCpuLocalDef;
            $(
                use $mp as $i;
            )*
            $vis struct $sn {
                $(
                    pub $i: <$i as FixedCpuLocalDef>::Type
                ),*
            }
            
            $vis fn $fn() {
                super::arch::_set_fixed_cpu_locals($sn {
                    $($i: <$i as FixedCpuLocalDef>::default()),*
                });
            }
        }
        $vis use $fclm::$sn;
        $vis use $fclm::$fn;
    }
}
def_cl_struct! {
    pub struct FixedCpuLocals (init fn init_fixed_cpu_locals, temp mod _fclm) {
        use crate::multitasking::fixedcpulocal::CPU_ID as CPU_ID,
        use crate::multitasking::interruptions::CURRENT_NOINTERRUPTIONS_STATE as CURRENT_NOINTERRUPTIONS_STATE,
    }
}
// pub struct FixedCpuLocals {
//     pub CPU_ID: CPU_ID::Type,
// }
// /* Call once per CPU, early on. */
// pub fn init_fixed_cpu_locals(){
//     // Store
//     arch::_set_fixed_cpu_locals(FixedCpuLocals {
//         cpu_id: cpu_id,
//         
//         current_nointerruptions_state: super::interruptions::FCLCurrentNIGuardDefault,
//     });
// }
#[inline(always)]
pub fn get_fixed_cpu_locals() -> &'static FixedCpuLocals {
    arch::_load_fixed_cpu_locals()
}

// CPU ID - each cpu is assigned an OS-derived "CPU ID" for easy sorting and identification and stuff
use core::sync::atomic::{AtomicUsize,Ordering};
static NEXT_CPU_ID: AtomicUsize = AtomicUsize::new(0);

fixed_cpu_local!(pub fixedcpulocal static CPU_ID: usize = NEXT_CPU_ID.fetch_add(1,Ordering::Acquire));

/* Get the CPU number for the local CPU.
    CPU numbers are assigned sequentially, so CPU 0 is the bootstrap processor, CPU 1 is the first AP to start, etc. */
#[inline(always)]
pub fn get_cpu_num() -> usize {
    *CPU_ID::get() // get_fixed_cpu_locals().cpu_id
}
