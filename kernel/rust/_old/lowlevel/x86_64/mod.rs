
mod gdt;
mod interrupts;
mod lowlevel;
pub mod multiboot;
pub mod context_switch;  // testing
pub mod smp;  // testing or smth
mod featureflags; use featureflags::init_msr; use featureflags::init_msr_ap;

pub use lowlevel::{halt, without_interrupts};
pub use smp::start_all_processors_xapic_acpi as start_all_processors;

use crate::coredrivers::system_apic;
