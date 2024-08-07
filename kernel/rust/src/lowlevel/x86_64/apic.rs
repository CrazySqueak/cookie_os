
use core::ptr::write_volatile;

use crate::memory::paging::global_pages::{KERNEL_PTABLE,KERNEL_PTABLE_VADDR};
use crate::memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
use crate::sync::SchedulerYield; use spin::RelaxStrategy;

pub const APIC_MAPPED_PADDR: usize = 0xFEE00_000;
pub const APIC_MAPPED_VADDR: usize = 0xFEE00_000 + KERNEL_PTABLE_VADDR;

pub struct APICReg<const ADDR:usize>;
impl<const ADDR:usize> APICReg<ADDR> {
    #[inline(always)]
    pub unsafe fn read() -> u32 {
        *(ADDR as *mut u32)
    }
    #[inline(always)]
    pub unsafe fn write(x: u32){
        write_volatile(ADDR as *mut u32, x)
    }
}

macro_rules! def_reg {
    ($name:ident, $offset:expr) => {
        pub type $name = APICReg<{APIC_MAPPED_VADDR + $offset}>;
    }
}

def_reg!(IcrLO , 0x300);
def_reg!(IcrHI , 0x310);

def_reg!(SVR, 0x0F0);

def_reg!(LocalID, 0x020);
impl LocalID {
    #[inline(always)]
    pub fn read_apic_id() -> u8 {
        ((unsafe { Self::read() } & 0xFF000000)>>24).try_into().unwrap()
    }
}

def_reg!(LvtCMCI, 0x2F0);
def_reg!(LvtTimer, 0x320);
def_reg!(LvtThermal, 0x330);
def_reg!(LvtPerfMon, 0x340);
def_reg!(LvtLint0, 0x350);
def_reg!(LvtLint1, 0x360);
def_reg!(LvtError, 0x370);

macro_rules! write64 {
    ($lo:ident,$hi:ident,$value:expr) => {
        $hi::write((($value&0xFFFFFFFF00000000)>>32) as u32);
        $lo::write( ($value&0x00000000FFFFFFFF)      as u32);
    }
}

pub unsafe fn send_icr<R: RelaxStrategy>(icr_value: u64){
    write64!(IcrLO,IcrHI,icr_value);
    // Small delay for APIC to register the command
    R::relax();
    // Wait until the Delivery Status becomes Idle
    while IcrLO::read()&0x1000 != 0 { R::relax(); }
}

/* Map the Local APIC into the page table as MMIO. 
    This is mapped in the bootstrap page table for us, but we need to map it ourselves. */
pub fn map_local_apic_mmio(){
    // Map page
    let apic_buf = KERNEL_PTABLE.allocate_at(APIC_MAPPED_VADDR, 0x1000).expect("Unable to map APIC MMIO!");
    apic_buf.set_base_addr(APIC_MAPPED_PADDR, PageFlags::new(TransitivePageFlags::empty(),MappingSpecificPageFlags::PINNED));
    apic_buf.leak();  // leak so that it doesn't get deallocated
}

/* Initialise the APIC, assuming that it's already been mapped (either as part of the bootstrap page table or in a later page table by using map_local_apic_mmio) */
pub unsafe fn enable_apic(){
    // Enable the APIC by setting bit 8 of the SVR
    let mut svr_value = SVR::read();
    svr_value |= 0b01_0000_0000;
    SVR::write(svr_value);
}
