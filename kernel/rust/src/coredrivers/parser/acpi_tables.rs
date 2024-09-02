use crate::memory::paging::global_pages::{MMIO_PTABLE,MMIO_PTABLE_VADDR,GlobalPageAllocation};  // technically not I/O but the "MMIO" page space is generally intended for hardware-specified stuff (such as the ACPI tables) anyway
use crate::memory::paging::{pageFlags,KALLOCATION_DYN_MMIO};
use acpi::handler::{AcpiHandler,PhysicalMapping as AcpiPhysicalMapping};
use alloc::{sync::Arc,vec::Vec};
use crate::sync::Mutex;

struct AcpiMemoryAllocation{ phys: usize, virt: usize, alloc: GlobalPageAllocation }

#[derive(Clone)]
pub struct AcpiMemoryMapper(Arc<Mutex<Vec<AcpiMemoryAllocation>>>);
impl AcpiMemoryMapper {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl AcpiHandler for AcpiMemoryMapper {
    unsafe fn map_physical_region<T>(&self, phys_addr: usize, size: usize) -> AcpiPhysicalMapping<Self,T> {
        // Map the requested address
        // (we don't have to touch our physical map as ACPI tables are marked as RESERVED by the bootloader, and thus aren't included as "free memory" by our physical allocator)
        // These have to be dynamically allocated as the ACPI parser constantly allocates them in non-page-sized amounts
        crate::logging::klog!(Info, ROOT, "acpi_pmap phys={:x} size={}", phys_addr, size);
        let allocation = MMIO_PTABLE.allocate_alignedoffset(size, KALLOCATION_DYN_MMIO, phys_addr).expect("Allocation for ACPI Tables failed?!");
        let virt_addr = allocation.base();
        allocation.set_base_addr(phys_addr, pageFlags!(m:PINNED));
        
        let virt_ptr = core::ptr::NonNull::new(virt_addr as *mut T).unwrap();
        let allocated_size = allocation.size();
        
        // Store the allocation somewhere
        self.0.lock().push(AcpiMemoryAllocation { phys: phys_addr, virt: virt_addr, alloc: allocation });
        // And return the requested mapping
        AcpiPhysicalMapping::new(phys_addr, virt_ptr,
                                 size, allocated_size,
                                 self.clone())
    }
    
    fn unmap_physical_region<T>(region: &AcpiPhysicalMapping<Self,T>) {
        // Acquire the lock
        let mut allocations = region.handler().0.lock();
        let position = allocations.iter().position(|a| a.phys == region.physical_start() && a.virt == (region.virtual_start().as_ptr() as usize)).expect("ACPI Allocation not found? Double free!");
        allocations.swap_remove(position);
    }
}

use acpi::AcpiError;
use acpi::platform::PlatformInfo;
use crate::coredrivers::parse_multiboot;
pub type AcpiTables = acpi::AcpiTables<AcpiMemoryMapper>;

pub fn parse_tables_multiboot() -> Option<Result<AcpiTables,AcpiError>> {
    let phys_addr = parse_multiboot::ACPI_RSDP_V2_PHYSADDR.or(*parse_multiboot::ACPI_RSDP_V1_PHYSADDR)?;
    Some(unsafe{parse_tables(phys_addr)})
}
pub unsafe fn parse_tables(rsdp_phys: usize) -> Result<AcpiTables,AcpiError> {
    AcpiTables::from_rsdp(AcpiMemoryMapper::new(), rsdp_phys)
}
