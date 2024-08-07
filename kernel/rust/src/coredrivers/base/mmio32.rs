
use core::ptr::{read_volatile,write_volatile};

pub trait MMIORegister {
    type RegisterSize;
}
pub trait MMIORegisterR : MMIORegister {
    fn read_raw(&mut self) -> Self::RegisterSize;
}
pub trait MMIORegisterW : MMIORegister {
    fn write_raw(&mut self, value: Self::RegisterSize);
}

pub struct MMIORegister32<const R: bool, const W: bool> { addr: usize }
impl<const R: bool, const W: bool> MMIORegister32<R,W> {
    pub const unsafe fn new(base: usize, off: usize) -> Self {
        Self { addr: base+off }
    }
}
impl<const R: bool, const W: bool> MMIORegister for MMIORegister32<R,W> {
    type RegisterSize = u32;
}
impl<const W: bool> MMIORegisterR for MMIORegister32<true,W> {
    #[inline(always)]
    fn read_raw(&mut self) -> Self::RegisterSize {
        unsafe{ read_volatile(self.addr as *const u32) }
    }
}
impl<const R: bool> MMIORegisterW for MMIORegister32<R,true> {
    #[inline(always)]
    fn write_raw(&mut self, value: Self::RegisterSize){
        unsafe{ write_volatile(self.addr as *mut u32, value) }
    }
}

// IO_DESCENDING -> read/write in descending order (HI first) rather than ascending order
pub struct MMIORegister64<const R: bool, const W: bool, const IO_DESCENDING:bool>{ addr_hi: usize, addr_lo: usize }
impl<const R: bool, const W: bool, const IO_DESCENDING:bool> MMIORegister64<R,W, IO_DESCENDING> {
    pub const unsafe fn new(base: usize, off_lo: usize, off_hi: usize) -> Self {
        Self { addr_lo: base+off_lo, addr_hi: base+off_hi }
    }
}
impl<const R: bool, const W: bool, const IO_DESCENDING:bool> MMIORegister for MMIORegister64<R,W,IO_DESCENDING> {
    type RegisterSize = u64;
}
impl<const W: bool, const IO_DESCENDING: bool> MMIORegister64<true,W,IO_DESCENDING> {
    #[inline(always)]
    fn rhi(&mut self) -> u32 { unsafe{ read_volatile(self.addr_hi as *const u32) } }
    #[inline(always)]
    fn rlo(&mut self) -> u32 { unsafe{ read_volatile(self.addr_lo as *const u32) } }
}
impl<const W: bool, const IO_DESCENDING: bool> MMIORegisterR for MMIORegister64<true,W,IO_DESCENDING> {
    #[inline(always)]
    fn read_raw(&mut self) -> Self::RegisterSize {
        let hi: u32; let lo: u32;
        if IO_DESCENDING { hi = self.rhi(); lo = self.rlo(); }
        else { lo = self.rlo(); hi = self.rhi(); }
        let hi: u64 = hi.into(); let lo: u64 = lo.into();
        hi<<32 + lo
    }
}
impl<const R: bool, const IO_DESCENDING: bool> MMIORegister64<R,true,IO_DESCENDING> {
    #[inline(always)]
    fn whi(&mut self, v:u32) { unsafe{ write_volatile(self.addr_hi as *mut u32, v) } }
    #[inline(always)]
    fn wlo(&mut self, v:u32) { unsafe{ write_volatile(self.addr_lo as *mut u32, v) } }
}
impl<const R: bool, const IO_DESCENDING: bool> MMIORegisterW for MMIORegister64<R,true,IO_DESCENDING> {
    #[inline(always)]
    fn write_raw(&mut self, value: Self::RegisterSize){
        let hi: u32 = (value>>32 & 0xFFFFFFFF).try_into().unwrap();
        let lo: u32 = (value     & 0xFFFFFFFF).try_into().unwrap();
        if IO_DESCENDING { self.whi(hi); self.wlo(lo); }
        else { self.wlo(lo); self.whi(hi); }
    }
}