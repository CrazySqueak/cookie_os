use super::util_mmio32::*;
use crate::logging::klog;

use crate::memory::paging::global_pages;
pub const LOCAL_APIC_MMIO_PHYS: usize = 0xFEE0_0000;
pub const LOCAL_APIC_MMIO_ROOT: usize = global_pages::MMIO_PTABLE_VADDR + LOCAL_APIC_MMIO_PHYS;

/* Map the local APIC to the memory-mapped IO global page table. */
pub fn map_local_apic_mmio() -> Option<global_pages::GlobalPageAllocation> {
    let buf = global_pages::MMIO_PTABLE.allocate_at(LOCAL_APIC_MMIO_ROOT, 0x400)?;
    use crate::memory::paging::{PageFlags,TransitivePageFlags,MappingSpecificPageFlags};
    buf.set_base_addr(LOCAL_APIC_MMIO_PHYS, PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::PINNED | MappingSpecificPageFlags::CACHE_WRITE_THROUGH | MappingSpecificPageFlags::CACHE_DISABLE));
    Some(buf)
}
// Local APIC
use crate::sync::cpulocal::CpuLocalLockedOption;
static _LOCAL_APIC: CpuLocalLockedOption<LocalAPIC> = CpuLocalLockedOption::new();
/* Initialise the CPU's local APIC */
pub fn init_local_apic(){
    klog!(Debug, COREDRIVERS_XAPIC, "Initialising local xAPIC");
    // Init local APIC
    _LOCAL_APIC.insert(unsafe { LocalAPIC::new(LOCAL_APIC_MMIO_ROOT) });
    
    // Configure APIC registers
    _LOCAL_APIC.mutate(|apic_o| {
        let apic = apic_o.as_mut().unwrap();
        
        // Read APIC ID
        let apic_id = apic.local_id.read_id();
        _LOCAL_APIC_ID.insert(apic_id);
        klog!(Info, COREDRIVERS_XAPIC, "Local APIC ID is {}.", apic_id);
        
        // Enable local APIC
        apic.siv.set_apic_enabled(true);
    });
}
/* Access the CPU's local APIC */
pub fn with_local_apic<R>(f: impl FnOnce(&mut LocalAPIC)->R)->R{
    _LOCAL_APIC.mutate(|apic_o| {
        let apic = apic_o.as_mut().expect("Cannot access local APIC, as this CPU's APIC isn't initialised yet!");
        f(apic)
    })
}

// APIC ID
pub type ApicID = u8;
static _LOCAL_APIC_ID: CpuLocalLockedOption<ApicID> = CpuLocalLockedOption::new();
/// Get the APIC Id for the given CPU
#[inline]
pub fn get_apic_id_for(cpu_num: usize) -> ApicID {
    _LOCAL_APIC_ID.get_for(cpu_num).lock().expect("Cannot get APIC ID for a CPU whose APIC isn't initialised yet!")
}

// = APIC IMPL =
pub struct LocalAPIC {
    pub local_id: LocalAPICId,
    pub siv: SpuriousInterruptVector,
    pub icr: InterruptCommandRegister,
}
impl LocalAPIC {
    unsafe fn new(base:usize)->Self { Self {
        local_id: LocalAPICId::new(base),
        siv: SpuriousInterruptVector::new(base),
        icr: InterruptCommandRegister::new(base),
    }}
}

pub struct LocalAPICId(MMIORegister32<true,true>);
impl LocalAPICId {
    unsafe fn new(base:usize)->Self { Self(MMIORegister32::new(base,0x020)) }
    pub fn read_id(&mut self) -> u8 {
        ((self.0.read_raw()&0xFF000000)>>24).try_into().unwrap()
    }
}

#[derive(Clone,Debug,Copy)]
pub enum IPIDestination {
    SelfOnly,
    EveryoneIncSelf,
    EveryoneButSelf,
    
    APICId(u8),
    Logical(u8),
}
impl IPIDestination {
    /* Convert from IPIDestination to (shorthand:2, mode:1, dest:8) */
    fn destructure(self) -> (u8, u8, u8) {
        use IPIDestination::*;
        match self {
            SelfOnly => (0b01,0,0),
            EveryoneIncSelf => (0b10,0,0),
            EveryoneButSelf => (0b11,0,0),
            
            APICId(id) => (0,0,id),
            Logical(x) => (0,1,x),
        }
    }
}
#[derive(Clone,Debug,Copy)]
pub enum InterProcessorInterrupt {
    Fixed(u8),
    SMI,
    NMI,
    INIT,
    /// Note: target address must be aligned to 4096 bytes and within the first 1MiB of memory
    SIPI(usize),
}
impl InterProcessorInterrupt {
    /* Turn an IPI into a (delivery_mode:3, vector:8) tuple. */
    fn destructure(self) -> (u8,u8) {
        use InterProcessorInterrupt::*;
        match self {
            Fixed(v) => (0b000,v),
            SMI => (0b010,0),
            NMI => (0b100,0),
            INIT => (0b101,0),
            SIPI(addr) => {
                // Convert target address to vector
                let tg_idx = addr/4096;
                assert!(addr%4096 == 0, "SIPI address must be aligned to 4096 bytes!");
                (0b110, tg_idx.try_into().expect("SIPI address must be within the first 1MiB of memory!"))
            },
        }
    }
}
pub struct InterruptCommandRegister(MMIORegister64<true,true,true>);
impl InterruptCommandRegister {
    unsafe fn new(base:usize)->Self { Self(MMIORegister64::new(base, 0x300,0x310)) }
    /* Send an IPI, blocking until it completes. */
    pub fn send_ipi(&mut self, ipi: InterProcessorInterrupt, dest: IPIDestination){
        klog!(Debug, COREDRIVERS_XAPIC, "Sending IPI {:?} to {:?}", ipi, dest);
        let (delivery_mode, ipi_vector) = ipi.destructure();
        let (dest_shorthand, dest_mode, dest_value) = dest.destructure();
        
        let mut ipi_value: u64 = 0;
        ipi_value |=  ipi_vector as u64 ;
        ipi_value |= (delivery_mode as u64)<<8;
        ipi_value |= (dest_mode as u64)<<11;
        ipi_value |= (dest_shorthand as u64)<<18;
        ipi_value |= (dest_value as u64)<<56;
        self.0.write_raw(ipi_value);
        
        // Wait for it to send
        todo!()
    }
}

const SIV_APIC_ENABLED: u32 = 0b01_0000_0000;  // bit 8
pub struct SpuriousInterruptVector(MMIORegister32<true,true>);
impl SpuriousInterruptVector {
    unsafe fn new(base:usize)->Self { Self(MMIORegister32::new(base,0x0F0)) }
    pub fn set_apic_enabled(&mut self, enable: bool){
        let mut siv = self.0.read_raw();
        if enable { siv |= SIV_APIC_ENABLED }
        else { siv &=! SIV_APIC_ENABLED };
        self.0.write_raw(siv);
    }
}

//static LOCAL_APIC_ID: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x020)};
//static SPURIOUS_INTERRUPT_VECTOR: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x0F0)};
//static INTERRUPT_COMMAND_REGISTER: APICRegister64<true,true,true> = unsafe{APICRegister64::new(LOCAL_APIC_MMIO_ROOT+

