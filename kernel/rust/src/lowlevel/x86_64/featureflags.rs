
use raw_cpuid::CpuId;
use x86_64::registers::model_specific::{Efer,EferFlags};
use x86_64::registers::control::{Cr4,Cr4Flags};

use crate::sync::KRwLock;
static _MSR_FLAGS: KRwLock<Option<(EferFlags,Cr4Flags)>> = KRwLock::new(None);

use crate::logging::klog;

macro_rules! incompatible {
    ($fn:ident, $bn:ident, $s:expr) => {
        $fn = true;
        let s = $s;
        klog!(Fatal, FEATURE_FLAGS, s);
        $bn.push(s);
    }
}

macro_rules! check_cpu_feature {
    ($in:ident, $fn:ident) => { $in.is_some() && $in.unwrap().$fn() };
    ($in:ident.$fn:ident) => { $in.is_some() && $in.unwrap().$fn() };
}

// this macro is messy - hopefully i don't have to touch it again
macro_rules! feature_check {
    // L2: "else warn/incompatible" - for features
    (feature=$featurename:literal name=$commonname:literal, $check:expr; set $success:expr; else warn) => {
        feature_check!(feature=$featurename name=$commonname, $check ; then set $success; else run {
            klog!(Info, FEATURE_FLAGS, "Compiled with feature '{}', but {} support is not available on this CPU. Continuing anyway.", $featurename, $commonname)
        })
    };
    (feature=$featurename:literal name=$commonname:literal, $check:expr; set $success:expr; else incompatible($failname:ident,$vecname:ident)) => {
        feature_check!(feature=$featurename name=$commonname, $check ; then set $success; else run {
            incompatible!($failname, $vecname, alloc::format!("Compiled with feature '{}', but {} support is unavailable on this CPU!", $featurename, $commonname).leak());
        })
    };
    (feature=$featurename:literal name=$commonname:literal, $check:expr; then set $success:expr; else run $failure:block) => {
        feature_check!(feature=$featurename, $check ; then {
            klog!(Debug, FEATURE_FLAGS, "Enabling {} support. (feature '{}')", $commonname, $featurename);
            $success
        } else $failure)
    };
    // L2: "else warn/incompatible" - for mandatory features
    (required name=$commonname:literal, $check:expr; set $success:expr; else warn) => {
        feature_check!(required name=$commonname, $check ; then set $success; else run {
            klog!(Info, FEATURE_FLAGS, "{} support is not available on this CPU. Continuing anyway.", $commonname)
        })
    };
    (required name=$commonname:literal, $check:expr; set $success:expr; else incompatible($failname:ident,$vecname:ident)) => {
        feature_check!(required name=$commonname, $check ; then set $success; else run {
            incompatible!($failname, $vecname, alloc::format!("{} support is required but not supported on this CPU!", $commonname).leak());
        })
    };
    (required name=$commonname:literal, $check:expr; then set $success:expr; else run $failure:block) => {
        feature_check!(true, $check ; then {
            klog!(Debug, FEATURE_FLAGS, "Enabling {} support.", $commonname);
            $success
        } else $failure)
    };
    
    // L1: feature= vs enabled=
    (feature=$featurename:literal, $check:expr; then $success:block else $failure:block) => {
        feature_check!(cfg!(feature=$featurename), $check ; then $success else $failure)
    };
    ($enabled:expr, $check:expr; then $success:block else $failure:block) => {
        if $enabled {
            if $check {
                $success
            } else {
                $failure
            }
        }
    };
}

pub fn init_msr(){
    // SAFETY: Care must be taken to set the flags correctly.
    //         Doing things wrong will fuck up memory safety and cause UB.
    unsafe {
        // if one or more options are incompatible, this is set
        // by setting this rather than immediately panicking, we allow people to see the full list of incompatible options instead of just one at a time
        let mut failed = false;
        let mut fail_reasons = alloc::vec::Vec::new();
        
        // Load CPUID and current flags
        let cpu_id = CpuId::new();
        let _cpuid_f    = cpu_id.get_feature_info()                              ; let cpuid_f = _cpuid_f.as_ref();
        let _cpuid_ef   = cpu_id.get_extended_feature_info()                     ; let cpuid_ef = _cpuid_ef.as_ref();
        let _cpuid_epfi = cpu_id.get_extended_processor_and_feature_identifiers(); let cpuid_epfi = _cpuid_epfi.as_ref();
        let _cpuid_pcfi = cpu_id.get_processor_capacity_feature_info()           ; let cpuid_pcfi = _cpuid_pcfi.as_ref();
        
        let mut eferflags = Efer::read();
        let mut cr4flags = Cr4::read();
        klog!(Debug, FEATURE_FLAGS, "Reading control registers: EFER={:?} CR4={:?}", eferflags, cr4flags);
        
        // Apply/check features
        // == EFER
        // No Execute in Page Tables
        feature_check!(feature="per_page_NXE_bit" name="per-page NX", check_cpu_feature!(cpuid_epfi.has_execute_disable) ; set eferflags |= EferFlags::NO_EXECUTE_ENABLE; else incompatible(failed,fail_reasons));
        
        // Translation Cache Extension (TCE)
        feature_check!(feature="enable_amd64_TCE" name="TCE", check_cpu_feature!(cpuid_epfi.has_tce) ; set eferflags |= EferFlags::TRANSLATION_CACHE_EXTENSION; else warn);
        
        // == CR4
        // Global Page Table Entries
        feature_check!(feature="page_global_bit" name="Global Page Mappings", check_cpu_feature!(cpuid_f.has_pge) ; set cr4flags |= Cr4Flags::PAGE_GLOBAL; else incompatible(failed, fail_reasons));
        
        // Supervisor Mode Execution Prevention (SMEP) - disables execution in kernel mode for pages that are accessible in user mode
        // Seems useful to have. I can always remove this if needed.
        feature_check!(required name="SMEP", check_cpu_feature!(cpuid_ef, has_smep); set cr4flags |= Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION; else warn);
        
        // == NO FLAG TO SET (just checks)
        // 1GiB Huge Pages
        feature_check!(feature="1G_huge_pages" name="1GiB Huge Page", check_cpu_feature!(cpuid_epfi, has_1gib_pages); set (); else incompatible(failed,fail_reasons));  // No flag to set here
        
        // APIC
        feature_check!(required name="APIC", check_cpu_feature!(cpuid_f, has_apic); set (); else incompatible(failed,fail_reasons));  // no flag to set
        
        // INVLPGB
        feature_check!(feature="enable_amd64_invlpgb" name="INVLPGB Instruction", check_cpu_feature!(cpuid_pcfi, has_invlpgb); set (); else warn);
        
        // == Handle success/failure
        if failed {
            panic!("One or more incompatible features were enabled!\r\n{}", fail_reasons.join("\r\n"));
        }
        
        // Save flags
        klog!(Debug, FEATURE_FLAGS, "Writing changes to control registers: EFER={:?} CR4={:?}", eferflags, cr4flags);
        Efer::write(eferflags);
        Cr4::write(cr4flags);
        
        // Store flags for use by APs
        let _=_MSR_FLAGS.write().insert((eferflags, cr4flags));
    }
}

pub fn init_msr_ap(){
    unsafe {
        use x86_64::registers::model_specific::{Efer,EferFlags};
        use x86_64::registers::control::{Cr4,Cr4Flags};
        let (eferflags, cr4flags) = _MSR_FLAGS.read().expect("init_msr_ap called before BSP set its own flags!");
        klog!(Debug, FEATURE_FLAGS, "Writing flags to control registers: EFER={:?} CR4={:?}", eferflags, cr4flags);
        Efer::write(eferflags);
        Cr4::write(cr4flags);
    }
}