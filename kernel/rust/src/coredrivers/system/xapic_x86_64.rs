use super::util_mmio32::*;

pub const LOCAL_APIC_MMIO_ROOT: usize = crate::memory::paging::global_pages::KERNEL_PTABLE_VADDR + 0xFEE0_0000;

// TODO: make local APIC cpu-local

pub struct LocalAPIC {
    local_id: LocalAPICId,
    icr: InterruptCommandRegister,
}
impl LocalAPIC {
    unsafe fn new()->Self { Self {
        local_id: LocalAPICId::new(),
        icr: InterruptCommandRegister::new(),
    }}
}

pub struct LocalAPICId(MMIORegister32<true,true>);
impl LocalAPICId {
    unsafe fn new()->Self { Self(MMIORegister32::new(LOCAL_APIC_MMIO_ROOT,0x020)) }
    pub fn read_id(&mut self) -> u8 {
        ((self.0.read_raw()&0xFF000000)>>24).try_into().unwrap()
    }
}

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
    unsafe fn new()->Self { Self(MMIORegister64::new(LOCAL_APIC_MMIO_ROOT, 0x300,0x310)) }
    /* Send an IPI, blocking until it completes. */
    pub fn send_ipi(&mut self, ipi: InterProcessorInterrupt, dest: IPIDestination){
        let (delivery_mode, ipi_vector) = ipi.destructure();
        let (dest_shorthand, dest_mode, dest_value) = dest.destructure();
        
        let mut ipi_value: u64 = 0;
        ipi_value |=  ipi_vector as u64 ;
        ipi_value |= (delivery_mode as u64)<<8;
        ipi_value |= (dest_mode as u64)<<11;
        ipi_value |= (dest_shorthand as u64)<<18;
        ipi_value |= (dest_mode as u64)<<56;
        self.0.write_raw(ipi_value);
        
        // Wait for it to send
        todo!()
    }
}

//static LOCAL_APIC_ID: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x020)};
//static SPURIOUS_INTERRUPT_VECTOR: APICRegister32<true,true> = unsafe{APICRegister32::new(LOCAL_APIC_MMIO_ROOT+0x0F0)};
//static INTERRUPT_COMMAND_REGISTER: APICRegister64<true,true,true> = unsafe{APICRegister64::new(LOCAL_APIC_MMIO_ROOT+
