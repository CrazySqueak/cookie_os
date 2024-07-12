use core::alloc::{Layout};
use alloc::vec::Vec;
use core::ptr::addr_of;

use lazy_static::lazy_static;

use crate::lowlevel::without_interrupts;
use crate::lowlevel::multiboot::MemoryMapEntry;
use crate::util::mutex_no_interrupts;

use core::fmt::write;
use crate::util::{LockedWrite,dbwriteserial};
//use crate::coredrivers::serial_uart::SERIAL1;

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

// ALLOCATOR
// This allocator works akin to a buddy allocator or something
#[repr(transparent)]
pub struct BuddyAllocator<const MAX_ORDER: usize, const MIN_SIZE: usize> {
    free_blocks: [Vec<usize>; MAX_ORDER],
}
impl<const MAX_ORDER: usize, const MIN_SIZE: usize> BuddyAllocator<MAX_ORDER,MIN_SIZE> {
    pub const MAX_ORDER: usize = MAX_ORDER;
    /* The size of a block of the given order. */
    pub const fn block_size(order: usize) -> usize {
        assert!(order <= MAX_ORDER, "Order out of bounds!");
        MIN_SIZE << order
    }
    
    /* Get the memory addresses of the memory address provided and its buddy, as a tuple of (lower buddy addr, higher buddy addr).
    One of these will be the memory address provided (if it is a valid address of a block). The other will be its buddy.*/
    pub fn buddies(order: usize, addr: usize) -> (usize, usize) {
        //dbwriteserial!("\t\tBuddies for {}:{:x} {:#32b}:\n", order, addr, addr);
        let parent_size = Self::block_size(order)<<1;  // we can't use order+1 because we might be at MAX_ORDER already
        let low_addr = (addr / parent_size) * parent_size;  // discard the middleness, as middleness isn't valid here
        let high_addr = low_addr + Self::block_size(order);
        //dbwriteserial!("\t\t\tLO =  {:x} {:#32b}\n", low_addr, low_addr);
        //dbwriteserial!("\t\t\tHI =  {:x} {:#32b}\n", high_addr, high_addr);
        (low_addr, high_addr)
    }
    
    /* Split a free block into two smaller blocks. */
    fn split(&mut self, order: usize, addr: usize) -> (usize, usize) {
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
    
    /* Merge a free block and its buddy, if possible. Otherwise, returns None.
        If the merge was successful, returns the address of the resulting block. */
    fn merge(&mut self, order: usize, addr: usize) -> Option<usize> {
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
            from_blocks.swap_remove(core::cmp::max(low_idx, high_idx));
            from_blocks.swap_remove(core::cmp::min(low_idx, high_idx));
        }
        
        // Add merged block to the free blocks list
        {
            let to_blocks = &mut self.free_blocks[to_order];
            to_blocks.push(to_addr);
        }
        
        Some(to_addr)
    }
    
    /* Add memory from [start,end) to this allocator's list of free blocks. */
    unsafe fn add_memory(&mut self, s: *const u8, e: *const u8){
        let mut start_u = s as usize;
        let mut end_u = e as usize;
        for order in (1..MAX_ORDER+1).rev() {  // MAX_ORDER -> 1 inc
            // Calculate the bounds of the block
            let split_order = order-1;
            let (block_start_addr, block_mid_addr) = Self::buddies(split_order, start_u);
            let block_end_excl = block_mid_addr + Self::block_size(split_order);
            
            // Chop to fit (if we're too big)
            if start_u < block_start_addr {
                // We need to add the extra area at the start (that the block doesn't cover)
                self.add_memory(start_u as *const u8, block_start_addr as *const u8);
                start_u = block_start_addr;
            }
            if end_u > block_end_excl {
                // We need to add the extra area at the end
                self.add_memory(block_end_excl as *const u8, end_u as *const u8);
                end_u = block_end_excl;
            }
            
            // What if the block is too big?
            if block_start_addr < start_u || block_end_excl > end_u { continue; }  // block is too big / overlaps the bounds
            
            // Add the block
            // (the low buddy is guaranteed to also count as a valid block for the order above)
            self.free_blocks[order].push(block_start_addr);
            return; // and return
        }
    }
}
pub type PFrameAllocator = BuddyAllocator<27,4096>;
mutex_no_interrupts!(LockedPFrameAllocator, PFrameAllocator);
lazy_static! {
    static ref PHYSMEM_ALLOCATOR: LockedPFrameAllocator = LockedPFrameAllocator::wraps(BuddyAllocator {
        free_blocks: core::array::from_fn(|_| Vec::new())
    });
}

use alloc::format;
impl<const MAX_ORDER: usize, const MIN_SIZE: usize> core::fmt::Debug for BuddyAllocator<MAX_ORDER,MIN_SIZE> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result{
        let mut fmt = f.debug_struct("BuddyAllocator - Free Blocks");
        for order in (0..MAX_ORDER).rev(){
            let bs = Self::block_size(order);
            
            let bs_str =      if bs > 0x100_0000_0000 { format!("{}TiB",bs>>40) }
                         else if bs > 0x____4000_0000 { format!("{}GiB",bs>>30) }
                         else if bs > 0x______10_0000 { format!("{}MiB",bs>>20) }
                         else if bs > 0x__________400 { format!("{}KiB",bs>>10) }
                         else { format!("{}B",bs) };
            
            fmt.field(&format!("Order {} ({} per block)", order, bs_str),
                      &self.free_blocks[order]);
        }
        fmt.finish()
    }
}

pub fn init_pmem(mmap: &Vec<MemoryMapEntry>){
    let (_, kend) = get_kernel_bounds();  // note: we ignore any memory before the kernel, its a tiny sliver (2MB tops) and isn't worth it
    dbwriteserial!("\tKernel ends @ {:x}\n", kend);
    PHYSMEM_ALLOCATOR.with_lock(|mut allocator|{
        for entry in mmap {
            dbwriteserial!("Checking PMem entry {:?}\n", entry);
            unsafe {
                if !entry.is_for_general_use() { continue; }
                dbwriteserial!("\tEntry is for general use.\n");
                let start_addr: usize = entry.base_addr.try_into().unwrap();
                let end_addr: usize = (entry.base_addr + entry.length).try_into().unwrap();
                dbwriteserial!("\tRange: [{:x},{:x})\n", start_addr, end_addr);
                if start_addr >= kend {
                    // after the kernel
                    dbwriteserial!("\tAfter the kernel\n");
                    dbwriteserial!("\tadd_memory({:x},{:x})\n", start_addr, end_addr);
                    allocator.add_memory(start_addr as *const u8, end_addr as *const u8);
                } else if end_addr > kend {
                    // intersecting the kernel
                    dbwriteserial!("\tIntersects the kernel\n");
                    dbwriteserial!("\tadd_memory({:x},{:x})\n", kend, end_addr);
                    allocator.add_memory(kend as *const u8, end_addr as *const u8);
                }
            }
        }
        dbwriteserial!("\nResult:{:#x?}\n", allocator);
    })
}

// ALLOCATIONS
#[derive(Debug)]
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

// Return the size block to allocate for the given layout, assuming that the given block is aligned by itself (which is the case for allocations).
fn calc_alloc_size(layout: &Layout) -> usize {
    layout.pad_to_align().size()
}

pub fn palloc(layout: Layout) -> Option<PhysicalMemoryAllocation> {
    dbwriteserial!("Requested to allocate physical memory for {:?}\n", layout);
    let alloc_size = calc_alloc_size(&layout);
    dbwriteserial!("\tAllocating {} bytes.\n", alloc_size);
    let (addr, order, size) = PHYSMEM_ALLOCATOR.with_lock(|mut allocator|{
        // Find best-sized order
        // Smallest order that is larger than or equal to the minimum size
        let order = match (0..PFrameAllocator::MAX_ORDER).position(|o| PFrameAllocator::block_size(o) >= alloc_size){ Some(x) => x, None => {dbwriteserial!("\tNo supported order is large enough to fulfill this request!\n"); None?}};
        dbwriteserial!("\tSelected order {}.\n", order);
        let addr = match req_block(&mut allocator, order){ Some(x) => x, None => {dbwriteserial!("\tNo free blocks of the requested order (or higher)!\n"); None?}};
        dbwriteserial!("\tAllocating addr {:x}, size: {} bytes.\n", addr, PFrameAllocator::block_size(order));
        let bidx = allocator.free_blocks[order].iter().position(|x| *x==addr).expect("Got invalid block!");
        allocator.free_blocks[order].swap_remove(bidx);
        
        Some((addr, order, PFrameAllocator::block_size(order)))
    })?;
    Some(PhysicalMemoryAllocation { 
        ptr: core::ptr::NonNull::new(addr as *mut u8).unwrap(),
        layout: layout,
        size: size,
        block: (order, addr),
    })
}
// get a block for the requested order
// (this does not remove it from the free list)
fn req_block(allocator: &mut PFrameAllocator, order: usize) -> Option<usize> {
    if order >= PFrameAllocator::MAX_ORDER { None }
    else if !allocator.free_blocks[order].is_empty() { Some(allocator.free_blocks[order][0]) }
    else {
        // Split a block of the next level up
        let splitblock = req_block(allocator, order+1)?;
        dbwriteserial!("\tSplitting {}:{:x} into two order {} blocks.\n", order+1, splitblock, order);
        let (lb, _) = allocator.split(order+1, splitblock);
        Some(lb)
    }
}

// PhysicalMemoryAllocations implement Drop, allowing them to be automatically freed once they are no longer referenced
impl core::ops::Drop for PhysicalMemoryAllocation {
    fn drop(&mut self){
        dbwriteserial!("Dropped {:?}.\n", self);
        PHYSMEM_ALLOCATOR.with_lock(|mut allocator|{
            let (order, addr) = self.block;
            
            // Return the block to the collection of free blocks
            allocator.free_blocks[order].push(addr);
            // And try to merge blocks until it's no longer possible
            let mut merge_addr = addr; let mut merge_order = order;
            while let Some(newaddr) = allocator.merge(merge_order, merge_addr){
                dbwriteserial!("\tMerged {}:{:x} -> {}:{:x}\n", merge_order, merge_addr, merge_order+1, newaddr);
                merge_order+=1; merge_addr = newaddr;
            }
        });
    }
}