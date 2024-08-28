
// TODO: Create public API that is as architecture-independent as possible

use core::ptr::addr_of;

use x86_64::VirtAddr;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use lazy_static::lazy_static;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

use alloc::boxed::Box;
use crate::sync::cpulocal::CpuLocalLockedOption;
pub struct GDTSegments {
    gdt: &'static GlobalDescriptorTable,
    tss: &'static TaskStateSegment,
    
    sg_kernel_code: SegmentSelector,
    sg_tss: SegmentSelector,
}
static _LOCAL_GDT: CpuLocalLockedOption<GDTSegments> = CpuLocalLockedOption::new();

fn _init_local_gdt(){
    // ===TSS
    let tss = Box::leak(Box::new(TaskStateSegment::new()));
    
    // double-fault stack
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
        const DF_STACK_SIZE: usize = 4096 * 8;
        let doublefaultstack = Box::new([0u8; DF_STACK_SIZE]);
        let stack_start = VirtAddr::from_ptr(Box::leak(doublefaultstack) as *mut u8);
        stack_start + (DF_STACK_SIZE.try_into().unwrap())
    };
    
    // ===GDT
    let gdt = Box::leak(Box::new(GlobalDescriptorTable::new()));
    let kernelcode = gdt.append(Descriptor::kernel_code_segment());
    let sg_tss = gdt.append(Descriptor::tss_segment(tss));
    
    _LOCAL_GDT.insert_and(GDTSegments { gdt, tss, sg_kernel_code: kernelcode, sg_tss: sg_tss }, |gdts|gdts.gdt.load());
}

pub fn init() {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{CS, Segment};
    _init_local_gdt();
    
    _LOCAL_GDT.inspect_unwrap(|gdts|unsafe {
        CS::set_reg(gdts.sg_kernel_code);
        load_tss(gdts.sg_tss);
    });
}