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
use crate::sync::cpulocal::{CpuLocalRWLockedItem,CpuLocalLockedOption};
use crate::sync::Mutex;
static _LOCAL_APIC: CpuLocalRWLockedItem<Option<LocalAPIC>> = CpuLocalRWLockedItem::new();
/* Initialise the CPU's local APIC */
pub fn init_local_apic(){
    klog!(Debug, COREDRIVERS_XAPIC, "Initialising local xAPIC");
    // Initialise APIC and configure APIC registers
    _LOCAL_APIC.mutate(|apic_o| {
        let apic = apic_o.insert(unsafe { LocalAPIC::new(LOCAL_APIC_MMIO_ROOT) });
        
        // Read APIC ID
        let apic_id = apic.config.lock().local_id.read_id();
        _LOCAL_APIC_ID.insert(apic_id);
        klog!(Info, COREDRIVERS_XAPIC, "Local APIC ID is {}.", apic_id);
        
        // Enable local APIC
        apic.config.lock().siv.set_apic_enabled(true);
    });
}
/* Access the CPU's local APIC */
pub fn with_local_apic<R>(f: impl FnOnce(&LocalAPIC)->R)->R{
    _LOCAL_APIC.inspect(|apic_o| {
        let apic = apic_o.as_ref().expect("Cannot access local APIC, as this CPU's APIC isn't initialised yet!");
        f(apic)
    })
}
/* Returns true if the local APIC has been initialised (e.g. using init_local_apic) */
pub fn is_local_apic_initialised() -> bool {
    _LOCAL_APIC.inspect(|apic_o|apic_o.is_some())
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
pub struct LocalAPICConfig {
    pub local_id: LocalAPICId,
    pub siv: SpuriousInterruptVector,
}
pub struct LocalVectorTable {
    pub timer: LVTTimer,
    pub cmci: LVTCMCI,
    pub lint0: LVTLINT0,
    pub lint1: LVTLINT1,
    pub error: LVTError,
    pub perfmon: LVTPerfMon,
    pub thermal: LVTThermalSensor,
}
pub struct LocalAPIC {
    pub config: Mutex<LocalAPICConfig>,
    pub lvt: Mutex<LocalVectorTable>,
    
    pub icr: Mutex<InterruptCommandRegister>,
    pub timer_counters: Mutex<TimerCounts>,
    
    pub eoi: EndOfInterrupt,
}
impl LocalAPIC {
    unsafe fn new(base:usize)->Self { Self {
        config: Mutex::new(LocalAPICConfig {
            local_id: LocalAPICId::new(base),
            siv: SpuriousInterruptVector::new(base),
        }),
        
        lvt: Mutex::new(LocalVectorTable {
            timer: LVTTimer::new(base),
            cmci: LVTCMCI::new(base),
            lint0: LVTLINT0::new(base),
            lint1: LVTLINT1::new(base),
            error: LVTError::new(base),
            perfmon: LVTPerfMon::new(base),
            thermal: LVTThermalSensor::new(base),
        }),
        
        icr: Mutex::new(InterruptCommandRegister::new(base)),
        timer_counters: Mutex::new(TimerCounts::new(base)),
        
        eoi: EndOfInterrupt::new(base),
    }}
}

pub struct LocalAPICId(MMIORegister32<true,false>);  // technically it's writable but doing so could confuse the kernel
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
    /* Send an IPI, blocking until it has sent. */
    pub fn send_ipi(&mut self, ipi: InterProcessorInterrupt, dest: IPIDestination){
        klog!(Debug, COREDRIVERS_XAPIC, "Sending IPI {:?} to {:?}", ipi, dest);
        self.send_ipi_raw(ipi, dest);
        
        // Wait for it to send
        use crate::multitasking::{yield_to_scheduler,SchedulerCommand};
        yield_to_scheduler(SchedulerCommand::PushBack);  // Ensure APIC has time to process our command
        while self.0.read_raw()&0x1000 != 0 { yield_to_scheduler(SchedulerCommand::SleepNTicks(1)); }  // wait until the IPI has sent
        // Done :)
    }
    /// Send an IPI without blocking or logging
    pub fn send_ipi_raw(&mut self, ipi: InterProcessorInterrupt, dest: IPIDestination){
        let (delivery_mode, ipi_vector) = ipi.destructure();
        let (dest_shorthand, dest_mode, dest_value) = dest.destructure();
        
        let mut ipi_value: u64 = 0;
        ipi_value |=  ipi_vector as u64 ;
        ipi_value |= (delivery_mode as u64)<<8;
        ipi_value |= (dest_mode as u64)<<11;
        ipi_value |= (dest_shorthand as u64)<<18;
        ipi_value |= (dest_value as u64)<<56;
        self.0.write_raw(ipi_value);
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
    pub fn set_spurious_vector(&mut self, vector: u8) {
        let mut siv = self.0.read_raw();
        // Clear and then set the spurious vector
        siv &=! 0x0FF; siv |= vector as u32;
        // Write
        self.0.write_raw(siv);
    }
}

pub struct LVTEntry<const OFFSET: usize, const DELIVERY_MODE_ENABLED: bool, const IS_LINT: bool, const IS_TIMER: bool>(MMIORegister32<true,true>);
impl<const OFFSET: usize, const DELIVERY_MODE_ENABLED: bool, const IS_LINT: bool, const IS_TIMER: bool> LVTEntry<OFFSET,DELIVERY_MODE_ENABLED, IS_LINT, IS_TIMER> {
    unsafe fn new(base:usize)->Self { Self(MMIORegister32::new(base, OFFSET)) }
    
    pub fn set_vector(&mut self, vector: u8){
        let mut entry = self.0.read_raw();
        entry &=! 0x0FF; entry |= vector as u32;
        self.0.write_raw(entry);
    }
    
    /* Returns true if an interrupt is waiting and has not been accepted yet. */
    pub fn is_waiting(&mut self) -> bool {
        (self.0.read_raw() & 0x0100) != 0  // Bit 12
    }
    
    pub fn set_masked(&mut self, mask: bool){
        const MASK_BIT: u32 = 0x010000;  // Bit 16
        let mut entry = self.0.read_raw();
        if mask { entry |= MASK_BIT }
        else { entry &=! MASK_BIT };
        self.0.write_raw(entry);
    }
}
impl<const OFFSET: usize, const IS_LINT: bool, const IS_TIMER: bool> LVTEntry<OFFSET,true, IS_LINT, IS_TIMER> {
    // that's a you problem tbh
    pub fn set_delivery_mode(&mut self, delivery_mode: u8){
        let delivery_bits = ((delivery_mode&0b0111) as u32)<<8;
        let mut entry = self.0.read_raw();
        entry &=! (0b0111<<8); entry |= delivery_bits;
        self.0.write_raw(entry);
    }
}
impl<const OFFSET: usize, const DELIVERY_MODE_ENABLED: bool, const IS_TIMER: bool> LVTEntry<OFFSET,DELIVERY_MODE_ENABLED,true,IS_TIMER> {
    // i'll do it eventually
}
pub enum TimerMode { OneShot, Repeating, TSCDeadline }
impl TimerMode { 
    pub fn as_bits(&self) -> u8 {
        use TimerMode::*;
        match self {
            OneShot => 0b00,
            Repeating => 0b01,
            TSCDeadline => 0b10,
        }
    }
}
impl<const OFFSET: usize, const DELIVERY_MODE_ENABLED: bool, const IS_LINT: bool> LVTEntry<OFFSET,DELIVERY_MODE_ENABLED,IS_LINT,true> {
    pub fn set_timer_mode(&mut self, timer_mode: TimerMode){
        let mode_bits = (timer_mode.as_bits() as u32)<<17;
        let mut entry = self.0.read_raw();
        entry &=! (0b011<<17); entry |= mode_bits;
        self.0.write_raw(entry);
    }
}

pub type LVTTimer = LVTEntry<0x320, false, false, true>;
pub type LVTCMCI = LVTEntry<0x2F0, true, false, false>;
pub type LVTLINT0 = LVTEntry<0x350, true, true, false>;
pub type LVTLINT1 = LVTEntry<0x360, true, true, false>;
pub type LVTError = LVTEntry<0x370, false, false, false>;
pub type LVTPerfMon = LVTEntry<0x340, true, false, false>;
pub type LVTThermalSensor = LVTEntry<0x330, true, false, false>;

pub struct TimerCounts{initial: MMIORegister32<true,true>, current: MMIORegister32<true,true>}
impl TimerCounts {
    unsafe fn new(base:usize)->Self { Self{initial:MMIORegister32::new(base,0x380),current:MMIORegister32::new(base,0x390)} }
    pub fn set_initial_count(&mut self, count: u32){
        self.initial.write_raw(count);
    }
    pub fn get_current_count(&mut self) -> u32 {
        self.current.read_raw()
    }
}

pub struct EndOfInterrupt(MMIORegister32<false,true>);
impl EndOfInterrupt {
    unsafe fn new(base:usize)->Self { Self(MMIORegister32::new(base,0x0B0)) }
    pub fn signal_eoi(&self){
        // This is safe because it's a simple signal register
        // it doesn't matter how many times it's called before or after
        // as long as the number of times it's called == the number of interrupts received
        unsafe {
            self.0.unchecked_write_raw(0);  // must be a zero
        }
    }
}

//static LOCAL_APIC_ID: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x020)};
//static SPURIOUS_INTERRUPT_VECTOR: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x0F0)};
//static INTERRUPT_COMMAND_REGISTER: APICRegister64<true,true,true> = unsafe{APICRegister64::new(LOCAL_APIC_MMIO_ROOT+

// I/O APIC
mod ioapicreg_sealed {
    pub type IOAPICRegID = u8;
    pub struct IOAPICRegDef<const R: bool, const W: bool>(IOAPICRegID);
    impl<const R:bool,const W:bool> IOAPICRegDef<R,W> { pub(super) const fn new(id: IOAPICRegID) -> Self { Self(id) } }
    pub trait IOAPICReg{ fn get_id(&self) -> IOAPICRegID; }
    impl<const R:bool,const W:bool> IOAPICReg for IOAPICRegDef<R,W> { fn get_id(&self) -> IOAPICRegID  { self.0 } }
    pub trait IOAPICRegR: IOAPICReg {} impl<const W: bool> IOAPICRegR for IOAPICRegDef<true,W>{}
    pub trait IOAPICRegW: IOAPICReg {} impl<const R: bool> IOAPICRegW for IOAPICRegDef<R,true>{}
    pub trait IOAPICRegRW: IOAPICRegR + IOAPICRegW{} impl IOAPICRegRW for IOAPICRegDef<true,true>{}
    
    pub type IOAPICRegDefRO = IOAPICRegDef<true,false>;
    pub type IOAPICRegDefWO = IOAPICRegDef<true,false>;
    pub type IOAPICRegDefRW = IOAPICRegDef<true,false>;
}
pub use ioapicreg_sealed::{IOAPICRegID, IOAPICRegR, IOAPICRegW, IOAPICRegRW};
use ioapicreg_sealed::{IOAPICRegDef,IOAPICRegDefRO,IOAPICRegDefWO,IOAPICRegDefRW};

pub struct IOAPIC {
    pub regselect: MMIORegister32<true,true>,
    pub data: MMIORegister32<true,true>,
}
impl IOAPIC {
    unsafe fn new(base: usize) -> Self { Self{regselect:MMIORegister32::new(base,0x00), data:MMIORegister32::new(base,0x10)} }
    fn select_register_raw(&mut self, reg: IOAPICRegID){
        self.regselect.write_raw(reg.into());  // bits 8-31 are reserved
    }
    fn read(&mut self, reg: &dyn IOAPICRegR) -> u32 {
        self.select_register_raw(reg.get_id());
        self.data.read_raw()
    }
    fn write(&mut self, reg: &dyn IOAPICRegW, data: u32) {
        self.select_register_raw(reg.get_id());
        self.data.write_raw(data);
    }
    fn read_modify_write(&mut self, reg: &dyn IOAPICRegRW, mutator: impl FnOnce(u32)->u32){
        self.select_register_raw(reg.get_id());
        let value = self.data.read_raw();
        let value = mutator(value);
        self.data.write_raw(value);
    }
    
    // ID
    pub const IOAPICID: IOAPICRegDefRO = IOAPICRegDefRO::new(0x00);
    pub fn get_ioapic_id(&mut self) -> u8 {
        // Bits [24,27] = id
        ((self.read(&Self::IOAPICID)&0x0F00_0000)>>24).try_into().unwrap()
    }
    // VER
    pub const IOAPICVER: IOAPICRegDefRO = IOAPICRegDefRO::new(0x01);
    pub fn get_max_redirection_entry(&mut self) -> u8 {
        // Bits [16,23]
        ((self.read(&Self::IOAPICVER)&0x00FF_0000)>>16).try_into().unwrap()
    }
}
