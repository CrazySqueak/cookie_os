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
const FRAME_0_SIZE: usize = 4096;
const FRAMES_PER_LEVEL: usize = 512;
const PAGE_FRAME_SIZES: [usize; 4] = [FRAME_0_SIZE, FRAME_0_SIZE*FRAMES_PER_LEVEL, FRAME_0_SIZE*FRAMES_PER_LEVEL*FRAMES_PER_LEVEL, FRAME_0_SIZE*FRAMES_PER_LEVEL*FRAMES_PER_LEVEL*FRAMES_PER_LEVEL];

// stored as an array of vecs of vecs
// array  - each item is a different order
// vec L1 - each item is a clique of buddies
// vec L2 - each item is a buddy, a specific frame of memory
pub type PhysicalMemoryAllocator = [Vec<Vec<usize>>; PAGE_FRAME_SIZES.len()];
mutex_no_interrupts!(LockedPhysicalMemoryAllocator, PhysicalMemoryAllocator);
fn find_buddies_idx(alloc: &PhysicalMemoryAllocator, order: usize, ptr: usize) -> Option<usize> {
    let frame_size = PAGE_FRAME_SIZES[order];
    let buddy_id: usize = ptr / frame_size;
    
    alloc[order].iter().position(|x| x[0]/frame_size == buddy_id)
}

lazy_static! {
    static ref PHYS_MEM_ALLOCATIONS: LockedPhysicalMemoryAllocator = LockedPhysicalMemoryAllocator::wraps(
            core::array::from_fn(|_| Vec::new()));
}

/* Split a page frame into frames of the next order down.
    Note: Once this is done, the ordering of the frames may change, due to the use of swap_remove.*/
fn split_frame(alloc: &mut PhysicalMemoryAllocator, order: usize, ptr: usize) {
    let to_order = order.checked_sub(1).expect("Cannot split a frame of order 0!");
    
    // Remove original frame
    let old_ptr_group_idx = find_buddies_idx(alloc, order, ptr).expect("Cannot split: Pointer is not free!");
    let old_ptr_group = &mut alloc[order][old_ptr_group_idx];
    let old_idx = old_ptr_group.iter().position(|x| *x==ptr).expect("Cannot split: Pointer is not free!");
    let old_ptr = old_ptr_group.swap_remove(old_idx);
    // And remove the group if it's now empty
    let should_remove = old_ptr_group.is_empty();
    if should_remove {alloc[order].swap_remove(old_ptr_group_idx);}
    
    // create new group for children
    // children are guaranteed to require a new group 
    // as a group consists of frames that belong to the same parent
    alloc[to_order].push(Vec::<usize>::with_capacity(FRAMES_PER_LEVEL));
    let new_group_idx = alloc[to_order].len()-1;
    let new_vec = &mut alloc[to_order][new_group_idx];
    
    // Create split children
    let next_idx = new_vec.len();
    let frame_size: usize = PAGE_FRAME_SIZES[to_order];
    for i in 0..FRAMES_PER_LEVEL {
        new_vec.push(old_ptr + (frame_size * i));
    }
}
/* Attempt to merge frames starting with the one at the given index, into a frame of the next order up.
Merging will only succeed if all 512 buddy frames of the current order are free. Otherwise, this function returns None.
Returns: The pointer to the frame of the next order up, if successful. Otherwise, returns None.*/
fn try_merge_frames(alloc: &mut PhysicalMemoryAllocator, order: usize, ptr: usize) -> Option<usize>{
    let to_order = order + 1;
    if to_order >= PAGE_FRAME_SIZES.len() { panic!("Cannot go higher than the maximum order!"); }
    
    // Find the old group
    let old_ptr_group_idx = find_buddies_idx(alloc, order, ptr)?;
    let old_ptr_group = &mut alloc[order][old_ptr_group_idx];
    if old_ptr_group.len() < FRAMES_PER_LEVEL { return None; }
    // We've got all 512, so drop the group
    alloc[order].swap_remove(old_ptr_group_idx);
    
    // Get the pointer as the next order up
    let new_ptr = ptr - (ptr%FRAMES_PER_LEVEL);
    // Find the new group
    let new_group_opt = find_buddies_idx(alloc, to_order, new_ptr).map(|x| &mut alloc[order][x]);
    // (or create it)
    let new_group = match new_group_opt {
        Some(x) => x,
        None => {
            alloc[to_order].push(Vec::<usize>::with_capacity(FRAMES_PER_LEVEL));
            let new_idx = alloc[to_order].len();
            &mut alloc[to_order][new_idx]
        }
    };
    
    // Add the new pointer to the new group
    new_group.push(new_ptr);
    // And return
    Some(new_ptr)
}

pub fn init_pmem(mmap: &Vec<MemoryMapEntry>){
    let (_, kend) = get_kernel_bounds();  // note: we ignore any memory before the kernel, its a tiny sliver (2MB tops) and isn't worth it
    write!(SERIAL1, "\n\tKernel ends @ {:x}", kend);
    //without_interrupts(||{
    //    let mut allocator = PHYSMEM_ALLOCATOR.lock();
    //    for entry in mmap {
    //        write!(SERIAL1, "\nChecking PMem entry {:?}", entry);
    //        unsafe {
    //            if !entry.is_for_general_use() { continue; }
    //            write!(SERIAL1, "\n\tEntry is for general use.");
    //            let start_addr: usize = entry.base_addr.try_into().unwrap();
    //            let end_addr: usize = (entry.base_addr + entry.length).try_into().unwrap();
    //            write!(SERIAL1, "\n\tRange: [{:x},{:x})", start_addr, end_addr);
    //            if start_addr >= kend {
    //                // after the kernel
    //                write!(SERIAL1, "\n\tAfter the kernel");
    //                allocator.add_to_heap(start_addr, end_addr);
    //            } else if end_addr > kend {
    //                // intersecting the kernel
    //                write!(SERIAL1, "\n\tIntersects the kernel");
    //                allocator.add_to_heap(kend, end_addr);
    //            }
    //        }
    //    }
    //})
}

use core::cmp::max;
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