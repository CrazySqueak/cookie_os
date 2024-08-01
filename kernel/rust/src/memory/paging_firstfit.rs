
use alloc::vec::Vec;
use alloc::boxed::Box;

use crate::logging::klog;
use super::*;

// Multi-Level First-Fit
pub struct MLFFAllocator<ST: PageFrameAllocator, PT: IPageTable, const SUBTABLES: bool, const HUGEPAGES: bool> {
    page_table: PT,
    
    suballocators: [Option<Box<ST>>; 512],  // TODO: NPAGES
    // Each addr is 2 bits: 00 = empty, 01 = occupied by table, 10 = occupied by table (half full), 11 = full / occupied by page
    availability_bitmap: [u8; 128],
}

impl<ST, PT: IPageTable, const SUBTABLES: bool, const HUGEPAGES: bool> MLFFAllocator<ST,PT,SUBTABLES,HUGEPAGES>
  where ST: PageFrameAllocator {
    fn get_subtable_always(&mut self, idx: usize) -> &mut Box<ST> {
        if let Some(ref mut subtable) = self.suballocators[idx] {
            return subtable;
        } else {
            // Create new allocator
            let new_st = self.suballocators[idx].insert(Self::__inst_subtable());
            // Add to page table
            Self::__point_to_subtable(&mut self.page_table, idx, new_st);
            // And return
            new_st
        }
    }
    
    fn __inst_subtable() -> Box<ST> {
        Box::new(ST::new())
    }
    fn __point_to_subtable<'a>(page_table: &'a mut PT, idx: usize, new_st: &'a Box<ST>){
        // SAFETY: The suballocator we take a reference to is owned by us. Therefore, it will not be freed unless we are freed, in which case the page table is also being freed.
        unsafe {
            page_table.set_subtable_addr_from_allocator(idx, &**new_st);
        }
    }
    
    fn get_availability(&self, i: usize) -> u8 {
        // 1122_3344 5566_7788
        (self.availability_bitmap[i/4] >> (6-(2*(i%4)))) & 0b11u8
    }
    fn set_availability(&mut self, idx: usize, value: u8){
        let lsh = 6-(2*(idx%4));
        self.availability_bitmap[idx/4] &= !(0b11u8 << lsh); // clear
        self.availability_bitmap[idx/4] |= value << lsh;  // set
    }
    fn refresh_availability(&mut self, idx: usize){
        let availability = 
            if let Some(alloc) = &self.suballocators[idx] {  // sub-table
                assert!(SUBTABLES);  // let statements in this position are unstable so fuck me i guess
                let npages_used = alloc.get_num_pages_used();
                if npages_used >= ST::NPAGES && alloc.is_full() {
                    0b11u8  // full
                } else if npages_used > ST::NPAGES/2 {
                    0b10u8  // more than half full
                } else {
                    // less than half full
                    0b01u8
                }
            } else {  // page or empty
                if self.page_table.is_unused(idx) {
                    0b00u8  // empty
                } else {
                    0b11u8  // occupied by huge page
                }
            }
        ;
        self.set_availability(idx, availability)
    }
    
    // allocates indexes [start, end)
    fn _alloc_contiguous(&mut self, start: usize, end: usize) -> Vec<PAllocItem> {
        klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Attempting contiguous allocation from [{},{})", start, end);
        let mut allocs = Vec::<PAllocItem>::new();
        for idx in start..end {
            // Allocate huge pages or subtables depending on if it's necessary
            assert!(self.get_availability(idx) == 0b00u8);
            if HUGEPAGES {
                // Reserve place
                self.page_table.reserve(idx);
                // Add huge page allocation to list
                allocs.push(PAllocItem::Page { index: idx, offset: (idx-start)*Self::PAGE_SIZE });
            } else if SUBTABLES {
                // Sub-tables
                let subtable = self.get_subtable_always(idx);
                if let Some(allocation) = subtable.allocate(Self::PAGE_SIZE) {
                    // Add allocation to list
                    allocs.push(PAllocItem::SubTable { index: idx, offset: (idx-start)*Self::PAGE_SIZE, alloc: allocation });
                } else {
                    panic!("This should never happen! allocation failed but did not match availability_bitmap!");
                }
            } else {
                panic!("Cannot have both SUBTABLES and HUGEPAGES set to false!");
            }
            self.refresh_availability(idx);
        };
        allocs
    }
    fn _alloc_rem(&mut self, idx: usize, inner_offset: usize, size: usize) -> Option<PartialPageAllocation> {
        assert!(SUBTABLES);
        let required_availability: u8 = if size >= Self::PAGE_SIZE/2 { 0b01u8 } else { 0b10u8 };  // Only test half-full ones if our remainder is small enough
        if idx < Self::NPAGES && self.get_availability(idx) <= required_availability {
            klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Attempting remainder allocation @ index={}, size={:x}, inner_offset={:x}", idx, size, inner_offset);
            // allocate remainder
            if let Some(allocation) = self.get_subtable_always(idx).allocate_at(inner_offset, size) {
                self.refresh_availability(idx);
                return Some(allocation);
            }
        };
        None
    }
    fn _alloc_inside(&mut self, idx: usize, size: usize) -> Option<PartialPageAllocation> {
        assert!(SUBTABLES);
        if idx < Self::NPAGES && self.get_availability(idx) < 0b11u8 {
            klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Attempting smaller allocation @ idx={}, size={:x}", idx, size);
            if let Some(allocation) = self.get_subtable_always(idx).allocate(size) {
                self.refresh_availability(idx);
                return Some(allocation);
            }
        };
        None
    }
    
    fn _build_allocation(&self, mut contig_result: Vec<PAllocItem>, rem_result: Option<(PAllocItem,usize)>) -> PartialPageAllocation {
        //let (mut huge_allocs, mut sub_allocs) = contig_result;
        
        // Offset if needed (and add remainder to sub allocs)
        if let Some((rem_alloc,offset_by)) = rem_result {
            // Increase offsets (if applicable)
            if offset_by != 0 {
                for item in contig_result.iter_mut() {
                    let offset = item.offset_mut();
                    *offset += offset_by;
                }
            }
            // Push remainder
            contig_result.push(rem_alloc);
            // Ensure result is sorted
            contig_result.sort_by_key(|i| i.offset());
        }
        
        PartialPageAllocation::new(contig_result, Self::PAGE_SIZE)
    }
}
impl<ST, PT: IPageTable, const SUBTABLES: bool, const HUGEPAGES: bool> PageFrameAllocatorImpl for MLFFAllocator<ST,PT,SUBTABLES,HUGEPAGES>
  where ST: PageFrameAllocator {
    const NPAGES: usize = PT::NPAGES;
    const PAGE_SIZE: usize = if SUBTABLES { ST::PAGE_SIZE * ST::NPAGES } else { 4096 };
    type PageTableType = PT;
    type SubAllocType = ST;
    
    fn new() -> Self {
        Self {
            page_table: PT::new(),
            
            suballocators: [const{None}; 512],
            availability_bitmap: [0; 128],
        }
    }
    
    fn get_num_pages_used(&self) -> usize {
        self.page_table.get_num_pages_used()
    }
    fn is_full(&self) -> bool {
        for av in &self.availability_bitmap {
            if *av != 0xFFu8 { return false; }  // If there's any space at all in sub-pages, then we're not full
        }
        true
    }
    fn get_page_table_ptr(&self) -> *const Self::PageTableType {
        core::ptr::addr_of!(self.page_table)
    }
    fn get_page_table_mut(&mut self) -> &mut Self::PageTableType {
        &mut self.page_table
    }
    fn get_suballocator_mut(&mut self, index: usize) -> Option<&mut Self::SubAllocType>{
        if !SUBTABLES { return None; }
        self.suballocators[index].as_deref_mut()
    }
    
    fn allocate(&mut self, size: usize) -> Option<PartialPageAllocation> {
        // We only support a non-page-sized remainder if we support sub-tables (as page frames cannot be divided)
        let pages = if SUBTABLES { size / Self::PAGE_SIZE } else { size.div_ceil(Self::PAGE_SIZE) };
        let remainder = if SUBTABLES { size % Self::PAGE_SIZE } else { 0 };
        klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "allocate: addr=ANY pages={} rem={} search=[0,{})", pages, remainder, Self::NPAGES-pages+1);
        
        // TODO: Replace with something that's not effectively O(n^2)
        for offset in 0..(Self::NPAGES-pages+1) {
            'check: {
                if SUBTABLES && pages < 1 {  // No whole pages needed. We don't need this complex sh*t. Just pass it down to a suballocator who is better equipped to deal with this
                    let i = offset;
                    
                    let result = if let Some(alloc) = self._alloc_inside(i, remainder) {
                        PAllocItem::SubTable { index: i, offset: 0, alloc: alloc }
                    } else { break 'check; };
                    
                    klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Allocated {} bytes (page_size=0x{:x}) @ start={}", remainder, Self::PAGE_SIZE, i);
                    return Some(PartialPageAllocation::new(alloc::vec![result],Self::PAGE_SIZE));
                    
                } else {
                    let start = offset; let end = offset+pages;
                    
                    // Test contiguous middle section
                    for i in start..end { if self.get_availability(i) != 0b00u8 { break 'check; } };
                    
                    // Test remainder (and if successful, we've got it!)
                    let remainder_allocated = 'allocrem: {
                      if remainder != 0 && SUBTABLES {
                        // Allocating before the contig part is TODO as idk how to ensure it's page-aligned at the bottom level
                        //if let Some(alloc) = self._alloc_rem(start.wrapping_sub(1), Self::PAGE_SIZE-remainder, remainder){
                        //    break 'allocrem Some((PAllocSubAlloc{index:end,offset:0,alloc:alloc}, 0));  // (allocation, offset for contig part)
                        //}
                        // Allocating at the end is fine
                        if let Some(alloc) = self._alloc_rem(end, 0, remainder){
                            break 'allocrem Some((PAllocItem::SubTable{index:end,offset:pages*Self::PAGE_SIZE,alloc:alloc}, 0));   // (allocation, offset for contig part)
                        }
                        // cannot allocate remainder
                        break 'check;
                      }
                      // No remainder (or subtables not possible so already rounded up)
                      None
                    };
                    
                    // Allocate middle
                    let contig_result = self._alloc_contiguous(start, end);
                    
                    // Return allocation
                    klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Allocated {} pages (page_size=0x{:x}) + {} bytes @ start={}", pages, Self::PAGE_SIZE, remainder, offset);
                    return Some(self._build_allocation(contig_result,remainder_allocated));
                }
            };
        }
        // If we get here then sadly we've failed
        None
    }
    
    fn allocate_at(&mut self, addr: usize, size: usize) -> Option<PartialPageAllocation> {
        let addr = crop_addr(addr);  // discard upper bits so that index is correct
        // We can't have remainders less than PAGE_SIZE if we don't support subtables, so we round down and add to the size to make up the difference.
        let start_idx = addr / Self::PAGE_SIZE;
        let (start_rem, size) = if SUBTABLES { (addr % Self::PAGE_SIZE, size) } else { (0, size + (addr % Self::PAGE_SIZE)) };
        let pages = if SUBTABLES { size / Self::PAGE_SIZE } else { size.div_ceil(Self::PAGE_SIZE) };
        let remainder = if SUBTABLES { size % Self::PAGE_SIZE } else { 0 };
        let end = start_idx+pages;
        
        klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "allocate_at: offset=0x{:x} page_size=0x{:x} start={} pages={} rem={}", addr, Self::PAGE_SIZE, start_idx, pages, remainder);
        
        // Check that the main area is clear
        for i in start_idx..end {
            if self.get_availability(i) != 0b00u8 { klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Unable to allocate start={} pages={}: index {} is occupied.", start_idx, pages, i); return None; }
        }
        // Check that the remainder is clear (if applicable)
        let remainder_allocated = 'allocrem: { if remainder != 0 && SUBTABLES {
                //klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "remainder @{},{:x},{:x}? availability={}",end,0+start_rem,remainder, self.get_availability(end));
                //{ klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "{:?}", self.suballocators[end].as_ref().map(|a| a.get_num_pages_used())); }
                
                if let Some(alloc) = self._alloc_rem(end, 0+start_rem, remainder){
                    break 'allocrem Some((PAllocItem::SubTable{index:end,offset:pages*Self::PAGE_SIZE,alloc:alloc},0));
                }
            // failed
            klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Unable to allocate start={} pages={}: failed to allocate remainder.", start_idx, pages);
            return None;
            }
            // no remainder
            None
        };
        
        // Allocate main section
        let contig_result = self._alloc_contiguous(start_idx, end);
        
        // And return
        klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Allocated {} pages (page_size=0x{:x}) + {} bytes.", pages, Self::PAGE_SIZE, remainder);
        Some(self._build_allocation(contig_result,remainder_allocated))
    }
    
    fn split_page(&mut self, index: usize) -> Result<PartialPageAllocation,()> {
        if (!SUBTABLES) || (!HUGEPAGES) { return Err(()); }  // not supported
        if self.get_availability(index) != 0b11u8 { return Err(()); }  // not a huge page
        if let Some(_) = self.suballocators[index] { return Err(()); }  // already a subtable
        
        // Create new allocation
        let suballoc = self.suballocators[index].insert(Self::__inst_subtable());
        let allocation = suballoc.allocate(Self::PAGE_SIZE).unwrap();
        
        // Copy flags across
        let entry = self.page_table.get_entry(index);
        let subpt = suballoc.get_page_table_mut();
        let stflags = if let Ok((addr, flags)) = entry {
            for i in 0..ST::NPAGES {
                subpt.set_huge_addr(i, addr+(i*ST::PAGE_SIZE), flags);
            }
            flags
        } else if let Err(data) = entry {
            for i in 0..ST::NPAGES {
                subpt.set_absent(i, data);
            }
            PageFlags::new(TransitivePageFlags::empty(), MappingSpecificPageFlags::empty())
        } else { unreachable!() };
        
        // Point to new subtable
        Self::__point_to_subtable(&mut self.page_table, index, suballoc);
        self.refresh_availability(index);
        // SET FLAGS ON SUBTABLE YOU FUCKING IDIOT
        self.page_table.add_subtable_flags::<false>(index, &stflags);
        
        // Et voila!
        Ok(allocation)
    }
    
    unsafe fn put_global_table(&mut self, index: usize, phys_addr: usize, mut flags: PageFlags){
        assert!(SUBTABLES);
        assert!(self.get_availability(index) == 0b00u8);
        
        flags.mflags |= MappingSpecificPageFlags::GLOBAL;
        
        klog!(Debug, MEMORY_PAGING_ALLOCATOR_MLFF, "Adding global page @{} -> {:x} (flags={:?})", index, phys_addr, flags);
        
        self.page_table.set_subtable_addr(index, phys_addr);
        self.page_table.add_subtable_flags::<true>(index, &flags);
        // Set spot in availability bitmap, to ensure that it isn't overwritten
        self.set_availability(index, 0b11u8);
    }
}
