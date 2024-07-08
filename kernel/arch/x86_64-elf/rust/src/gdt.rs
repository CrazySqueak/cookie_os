
// TODO: Create public API that is as architecture-independent as possible

use core::ptr::addr_of;

use x86_64::VirtAddr;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use lazy_static::lazy_static;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

            let stack_start = VirtAddr::from_ptr(unsafe { addr_of!(STACK) });
            let stack_end = stack_start + (STACK_SIZE as u64);
            stack_end
        };
        tss
    };
}

lazy_static! {
    static ref GDT_AND_FRIENDS: (GlobalDescriptorTable, SegmentSelector, SegmentSelector) = {
        let mut gdt = GlobalDescriptorTable::new();
        // breaking changes abound! I'm pretty sure intel CPUs haven't suddenly changed in the last 3 years so uhh. This one's on you.
        // *hits x86_64's maintainer over the head with https://semver.org/ *
        // *hits tutorial writer over the head for not providing the version of x86_64 they explain how to use*
        let kcs = gdt.append(Descriptor::kernel_code_segment());
        let tss = gdt.append(Descriptor::tss_segment(&TSS));
        (gdt, kcs, tss)
    };
    static ref GDT: &'static GlobalDescriptorTable = &GDT_AND_FRIENDS.0;
    static ref GDT_KERNEL_CODE: SegmentSelector = GDT_AND_FRIENDS.1;
    static ref GDT_TSS: SegmentSelector = GDT_AND_FRIENDS.2;
}

pub fn init() {
    use x86_64::instructions::tables::load_tss;
    use x86_64::instructions::segmentation::{CS, Segment};
    
    GDT.load();
    
    unsafe {
        CS::set_reg(*GDT_KERNEL_CODE);
        load_tss(*GDT_TSS);
    }
}