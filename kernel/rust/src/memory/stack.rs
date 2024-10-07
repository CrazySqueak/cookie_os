use core::ptr::addr_of;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering::{Acquire, Relaxed};
use crate::memory::alloc_util::{AnyAllocatedStack, HeapReclaimableAllocatedStack};

// BOOTSTRAP STACKS
extern "sysv64" {
    static kstack_top: u8;
    static kstack_bottom: u8;
}
static BOOTSTRAP_STACK_CLAIMED: AtomicBool = AtomicBool::new(false);
/// Panics if the stack was already claimed.
pub fn claim_bsp_boostrap_stack() -> impl AnyAllocatedStack {
    match BOOTSTRAP_STACK_CLAIMED.compare_exchange(false, true, Acquire, Relaxed) {
        Ok(_)=>{},
        Err(_) => panic!("BSP Bootstrap stack has already been claimed!"),
    }
    // SAFETY: This compare_exchange guarantees we may only acquire ownership of the stack once
    //          kstack_top and kstack_bottom are not values but symbols, so acquiring the address of them should be fine.
    unsafe {
        // Note: some named the *highest* address `kstack_top` instead of `kstack_bottom`
        //          so we have to switch them around here for our use
        let kstack_top_addr = addr_of!(kstack_bottom);
        let kstack_bot_addr = addr_of!(kstack_top);
        HeapReclaimableAllocatedStack::new(
            kstack_top_addr,
            kstack_bot_addr,
        )
    }
}

// fixme
#[no_mangle]
#[used]
static next_processor_stack: u8 = 0xaa;  // intellij shut the fuck up this is no_mangle so I can't obey naming conventions (also it's a placeholder anyway)