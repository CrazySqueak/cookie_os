use core::alloc::{Layout};
use alloc::vec::Vec;
use core::ptr::addr_of;

use lazy_static::lazy_static;

use crate::lowlevel::without_interrupts;
use crate::lowlevel::multiboot::MemoryMapEntry;
use crate::util::mutex_no_interrupts;

use core::fmt::write;
use crate::util::LockedWrite;
use crate::coredrivers::serial_uart::SERIAL1;

// Memory occupied by the kernel (set by the linker script)
extern "C" {
    static kernel_phys_start: u8;
    static kernel_phys_end: u8;
}
/* Get the kernel's start and end in physical memory. (start inclusive, end exclusive)
This covers the executable, bss, etc. sections, but does not cover any memory that was dynamically allocated afterwards.
(N.B. since the initial kernel heap is defined in the .bss section, that IS included, but if the heap is expanded afterwards then the expanded parts won't be included.) */
pub fn get_kernel_bounds() -> (usize, usize) {
    unsafe {
        (addr_of!(kernel_phys_start) as usize, addr_of!(kernel_phys_end) as usize)
    }
}

// ALLOCATIONS
pub struct PhysicalMemoryAllocation {
    ptr: core::ptr::NonNull<u8>,
    layout: Layout,
    size: usize,
    
    block: (usize, usize),  // (order, addr)
}
impl PhysicalMemoryAllocation {
    pub fn get_ptr(&self) -> core::ptr::NonNull<u8> { self.ptr }
    pub fn get_size(&self) -> usize { self.size }
}
// Memory allocations cannot be copied nor cloned,
// as that would allow for use-after-free or double-free errors
// Physical memory allocations are managed by Rust's ownership rules
// (though it is still the responsibility of the kernel to ensure userspace processes
//      cannot access physical memory that has been freed or re-used)
impl !Copy for PhysicalMemoryAllocation{}
impl !Clone for PhysicalMemoryAllocation{}
impl !Sync for PhysicalMemoryAllocation{}

// ALLOCATOR
// This allocator works akin to a buddy allocator or something
#[derive(Debug)]
#[repr(transparent)]
pub struct BuddyAllocator<const MAX_ORDER: usize, const MIN_SIZE: usize> {
    free_blocks: [Vec<usize>; MAX_ORDER],
}
impl<const MAX_ORDER: usize, const MIN_SIZE: usize> BuddyAllocator<MAX_ORDER,MIN_SIZE> {
    /* The size of a block of the given order. */
    pub const fn block_size(order: usize) -> usize {
        assert!(order <= MAX_ORDER, "Order out of bounds!");
        MIN_SIZE << order
    }
    
    /* Get the memory addresses of the memory address provided and its buddy, as a tuple of (lower buddy addr, higher buddy addr).
    One of these will be the memory address provided (if it is a valid address of a block). The other will be its buddy.*/
    pub const fn buddies(order: usize, addr: usize) -> (usize, usize) {
        let low_addr = addr &!(Self::block_size(order)<<1);  // we can't call block_size(order+1) as there's no guarantee that order is not MAX_ORDER
        let high_addr = low_addr + Self::block_size(order);
        (low_addr, high_addr)
    }
    
    /* Split a free block into two smaller blocks. */
    pub fn split(&mut self, order: usize, addr: usize) -> (usize, usize) {
        let to_order = order.checked_sub(1).expect("Can't split a block of order 0!");
        let to_addrs = Self::buddies(to_order, addr);
        
        // Find and remove from the free blocks list
        {
            let from_blocks = &mut self.free_blocks[order];
            let from_idx = from_blocks.iter().position(|x| *x==addr).expect("Can't split a block that isn't free!");
            from_blocks.swap_remove(from_idx);
        }
        
        // Add split halves to the free blocks list
        {
            let to_blocks = &mut self.free_blocks[to_order];
            to_blocks.push(to_addrs.0); to_blocks.push(to_addrs.1);
        }
        
        to_addrs
    }
    
    /* Merge a free block and its buddy, if possible. Otherwise, returns None. */
    pub fn merge(&mut self, order: usize, addr: usize) -> Option<usize> {
        let to_order = order + 1;
        if to_order > MAX_ORDER { return None; }
        let (low_addr, high_addr) = Self::buddies(order, addr);
        let to_addr = low_addr;
        
        // Check that buddies are free
        {
            let from_blocks = &mut self.free_blocks[order];
            let low_idx = from_blocks.iter().position(|x| *x==low_addr)?;
            let high_idx = from_blocks.iter().position(|x| *x==high_addr)?;
            // And remove from the free blocks list
            // (we must do the highest index first, as if the highest index was at the end of the list then swap_remove-ing the lower index would break things)
            from_blocks.swap_remove(core::cmp::max(low_addr, high_addr));
            from_blocks.swap_remove(core::cmp::min(low_addr, high_addr));
        }
        
        // Add merged block to the free blocks list
        {
            let to_blocks = &mut self.free_blocks[to_order];
            to_blocks.push(to_addr);
        }
        
        Some(to_addr)
    }
    
    /* Add memory from [start,end) to this allocator's list of free blocks. */
    pub unsafe fn add_memory(&mut self, start: *const u8, end: *const u8){
        for order in (1..MAX_ORDER+1).rev() {  // MAX_ORDER -> 1 inc
            // Calculate the bounds of the block
            let split_order = order-1;
            let (block_start_addr, block_mid_addr) = Self::buddies(split_order, start as usize);
            let block_end_excl = block_mid_addr + Self::block_size(split_order);
            if block_start_addr < (start as usize) || block_end_excl > (end as usize) { continue; }  // block is too big / overlaps the bounds
            
            if (start as usize) < block_start_addr {
                // We need to add the extra area at the start (that the block doesn't cover)
                self.add_memory(start, block_start_addr as *const u8);
            }
            if (end as usize) > block_end_excl {
                // We need to add the extra area at the end
                self.add_memory(block_end_excl as *const u8, end);
            }
            
            // Add the block
            // (the low buddy is guaranteed to also count as a valid block for the order above)
            self.free_blocks[order].push(block_start_addr);
            return; // and return
        }
    }
}
mutex_no_interrupts!(LockedPFrameAllocator, BuddyAllocator<27,4096>);
lazy_static! {
    static ref PHYSMEM_ALLOCATOR: LockedPFrameAllocator = LockedPFrameAllocator::wraps(BuddyAllocator {
        free_blocks: core::array::from_fn(|_| Vec::new())
    });
}

pub fn init_pmem(mmap: &Vec<MemoryMapEntry>){
    let (_, kend) = get_kernel_bounds();  // note: we ignore any memory before the kernel, its a tiny sliver (2MB tops) and isn't worth it
    write!(SERIAL1, "\n\tKernel ends @ {:x}", kend);
    PHYSMEM_ALLOCATOR.with_lock(|mut allocator|{
        for entry in mmap {
            write!(SERIAL1, "\nChecking PMem entry {:?}", entry);
            unsafe {
                if !entry.is_for_general_use() { continue; }
                write!(SERIAL1, "\n\tEntry is for general use.");
                let start_addr: usize = entry.base_addr.try_into().unwrap();
                let end_addr: usize = (entry.base_addr + entry.length).try_into().unwrap();
                write!(SERIAL1, "\n\tRange: [{:x},{:x})", start_addr, end_addr);
                if start_addr >= kend {
                    // after the kernel
                    write!(SERIAL1, "\n\tAfter the kernel");
                    write!(SERIAL1, "\n\tadd_memory({:x},{:x})", start_addr, end_addr);
                    allocator.add_memory(start_addr as *const u8, end_addr as *const u8);
                } else if end_addr > kend {
                    // intersecting the kernel
                    write!(SERIAL1, "\n\tIntersects the kernel");
                    write!(SERIAL1, "\n\tadd_memory({:x},{:x})", kend, end_addr);
                    allocator.add_memory(kend as *const u8, end_addr as *const u8);
                }
            }
        }
        write!(SERIAL1, "\n\nResult:{:x?}", allocator);
    })
}

// Return the size block to allocate for the given layout, assuming that the given block is aligned by itself (which is the case for allocations).
fn calc_alloc_size(layout: &Layout) -> usize {
    layout.pad_to_align().size()
}

pub fn palloc(layout: Layout) -> Option<PhysicalMemoryAllocation> {
    //let (ptr,size) = without_interrupts(||{
    //    let mut allocator = PHYSMEM_ALLOCATOR.lock();
    //    let old_actual = allocator.stats_alloc_actual();
    //    let ptr = allocator.alloc(layout).ok()?;
    //    let size = allocator.stats_alloc_actual()-old_actual;
    //    Some((ptr,size))
    //})?;
    //Some(PhysicalMemoryAllocation {
    //    ptr: ptr,
    //    layout: layout,
    //    size: size,
    //})
    None
}
pub fn pfree(allocation: PhysicalMemoryAllocation){
    //without_interrupts(||{
    //    PHYSMEM_ALLOCATOR.lock().dealloc(allocation.ptr, allocation.layout)
    //})
}