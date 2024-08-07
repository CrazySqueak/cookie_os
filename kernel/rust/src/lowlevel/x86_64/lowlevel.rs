use core::arch::asm;

/* The offset between the kernel's virtual memory and the computer's physical memory.
    As this is a higher-half kernel, any memory mapped I/O, page table locations, etc. should be converted using this constant.
    (the kernel will always be mapped between the given physical memory and virtual memory)
    Note that converting userspace addresses by this constant will not end well, as they are mapped by their page table (and are not necessarily contiguous in physical memory.) */
//pub const HIGHER_HALF_OFFSET: usize = 0xFFFF800000000000;
// this is no longer true as the global pages system is in use instead

pub fn halt() -> ! {
    // SAFETY: This code does not modify memory besides disabling interupts, and does not return
    // it is an end point after which nothing more should happen
    // or something
    // i really shouldn't be allowed to use unsafe{} when I'm this tired lmao
    unsafe {
        asm!("cli"); // disable interrupts
        loop {
            asm!("hlt");  // halt
        }
    }
}

pub fn without_interrupts<R,F: FnOnce()->R>(f: F) -> R{
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(f)
}

macro_rules! incompatible {
    ($fn:ident, $bn:ident, $s:expr) => {
        $fn = true;
        let s = $s;
        klog!(Fatal, FEATURE_FLAGS, s);
        $bn.push(s);
    }
}

use crate::logging::klog;
pub fn init_msr(){
    // SAFETY: Care must be taken to set the flags correctly.
    //         Doing things wrong will fuck up memory safety and cause UB.
    unsafe {
        use raw_cpuid::CpuId;
        use x86_64::registers::model_specific::{Efer,EferFlags};
        use x86_64::registers::control::{Cr4,Cr4Flags};
        // if one or more options are incompatible, this is set
        // by setting this rather than immediately panicking, we allow people to see the full list of incompatible options instead of just one at a time
        let mut failed = false;
        let mut fail_reasons = alloc::vec::Vec::new();
        
        // Load CPUID and current flags
        let cpu_id = CpuId::new();
        let _cpuid_ef   = cpu_id.get_extended_feature_info()                     ; let cpuid_ef = _cpuid_ef.as_ref();
        let _cpuid_epfi = cpu_id.get_extended_processor_and_feature_identifiers(); let cpuid_epfi = _cpuid_epfi.as_ref();
        
        let mut eferflags = Efer::read();
        let mut cr4flags = Cr4::read();
        klog!(Debug, FEATURE_FLAGS, "Reading control registers: EFER={:?} CR4={:?}", eferflags, cr4flags);
        
        // Apply/check features
        // No Execute in Page Tables
        if cfg!(feature = "per_page_NXE_bit"){
            if cpuid_epfi.is_some() && cpuid_epfi.unwrap().has_execute_disable() {
                klog!(Debug, FEATURE_FLAGS, "Enabling per-page NX support.");
                eferflags |= EferFlags::NO_EXECUTE_ENABLE;
            } else {
                incompatible!(failed, fail_reasons, "Compiled with per-page NX support, but per-page NX is unavailable on this CPU!");
            }
        }
        
        // Global Page Table Entries
        if cfg!(feature = "page_global_bit"){
           klog!(Debug, FEATURE_FLAGS, "Enabling Global Page support.");
           cr4flags |= Cr4Flags::PAGE_GLOBAL;
        }
        
        // Supervisor Mode Execution Prevention (SMEP) - disables execution in kernel mode for pages that are accessible in user mode
        // Seems useful to have. I can always remove this if needed.
        if true {
            if cpuid_ef.is_some() && cpuid_ef.unwrap().has_smep() {
                klog!(Debug, FEATURE_FLAGS, "Enabling SMEP support.");
                cr4flags |= Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION;
            } else {
                klog!(Info, FEATURE_FLAGS, "Compiled with SMEP support, but SMEP is unavailable on this CPU.");
            }
        }
        
        // Translation Cache Extension (TCE)
        if cfg!(feature = "enable_amd64_TCE"){
            if cpuid_epfi.is_some() && cpuid_epfi.unwrap().has_tce() {
                klog!(Debug, FEATURE_FLAGS, "Enabling TCE.");
                eferflags |= EferFlags::TRANSLATION_CACHE_EXTENSION;
            } else {
                klog!(Info, FEATURE_FLAGS, "Compiled with TCE support, but TCE is unavailable on this CPU.");
            }
        }
        
        // 1GiB Huge Pages
        if cfg!(feature = "1G_huge_pages"){
            if cpuid_epfi.is_some() && cpuid_epfi.unwrap().has_1gib_pages() {
                klog!(Debug, FEATURE_FLAGS, "Enabling 1GiB Huge Page support.");
                // Note: no flag to set here (it's set in the page entries)
            } else {
                incompatible!(failed, fail_reasons, "Compiled with 1GiB Huge Page support, but 1GiB Huge Pages are unavailable on this CPU!");
            }
        }
        
        if failed {
            panic!("One or more incompatible features were enabled!\r\n{}", fail_reasons.join("\r\n"));
        }
        
        // Save flags
        klog!(Debug, FEATURE_FLAGS, "Writing changes to control registers: EFER={:?} CR4={:?}", eferflags, cr4flags);
        Efer::write(eferflags);
        Cr4::write(cr4flags);
    }
}