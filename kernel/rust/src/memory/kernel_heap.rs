
use buddy_system_allocator::LockedHeap;

extern "C" {
    // Provided by longmode.intel.asm (64-bit)
    pub static kheap_initial_addr: usize;
    pub static kheap_initial_size: usize;
}

#[global_allocator]
static KHEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();

pub fn init_kheap(){
    unsafe {
        KHEAP_ALLOCATOR.lock().init(kheap_initial_addr as usize,kheap_initial_size as usize);
    }
}