use alloc::vec::Vec;

#[repr(C)]
pub struct MultibootInfo {
    // These flags define which fields are present and which are null
    multiboot_flags: u32,
    
    mem_lower: u32,
    mem_upper: u32,
    
    boot_device: u32,
    
    cmdline: u32,
    
    mods_count: u32,
    mods_addr: u32,
    
    // no clue what the fuck to do with these
    syms_1: u32,
    syms_2: u32,
    syms_3: u32,
    syms_4: u32,
    
    // memory map
    mmap_length: u32,
    mmap_addr: u32,
    
    // TODO: add more as needed
    // spec: https://www.gnu.org/software/grub/manual/multiboot/html_node/Boot-information-format.html#Boot-information-format
}

const MBFLAG_MEM      : u32 = 0x0001;
const MBFLAG_BOOTDEV  : u32 = 0x0002;
const MBFLAG_CMDLINE  : u32 = 0x0004;
const MBFLAG_MODS     : u32 = 0x0008;
const MBFLAG_SYMS_AOUT: u32 = 0x0010;
const MBFLAG_SYMS_ELF : u32 = 0x0020;
const MBFLAG_MEMMAP   : u32 = 0x0040;

impl MultibootInfo {
    // Get a pointer to the multiboot-provided memory map, or None if not provided
    pub fn get_memmap_ptr(&self) -> Option<(*const MemoryMapEntry, usize)>{
        if (self.multiboot_flags & MBFLAG_MEMMAP) != 0 {
            let ptr = self.mmap_addr as *const MemoryMapEntry;
            let len = self.mmap_length as usize;
            Some((ptr,len))
        } else {None}
    }
    
    // Get a Vec of the entries in the MemoryMap. Each entry is copied into the Vec for memory safety and ease of use.
    // If no memory map is provided, then this returns None.
    // SAFETY: This function is gated by an immutable reference to MultibootInfo
    // It does not modify the MultibootInfo in any way, so as long as the standard borrowing rules are upheld, no UB should occur.
    pub fn get_memmap(&self) -> Option<Vec<MemoryMapEntry>> {
        if let Some((ptr, mm_len)) = self.get_memmap_ptr() {
            let mut offset: usize = 0;
            let mut entries: Vec<MemoryMapEntry> = Vec::new();
            unsafe {
                while offset < mm_len {
                    let mm_entry = ptr.byte_add(offset) as *const MemoryMapEntry;
                    
                    entries.push((*mm_entry).clone());  // add to vec
                    offset += (*mm_entry).entry_size as usize;  // Seek to next entry
                }
            };
            Some(entries)
        } else {None}
    }
}

#[derive(Clone,Debug)]
#[repr(C,packed)]
pub struct MemoryMapEntry {
    entry_size: u32,
    base_addr_low: u32,
    base_addr_high: u32,
    mem_length_low: u32,
    mem_length_high: u32,
    mem_type: u32,
}
impl MemoryMapEntry{
    // get the base address for this area
    pub fn get_mem_addr(&self) -> *mut u8 {
        (((self.base_addr_high as usize) << 32) | (self.base_addr_low as usize)) as *mut u8
    }
    // get the length of this area
    pub fn get_mem_length(&self) -> u64 {
        (((self.mem_length_high as u64) << 32) | (self.mem_length_low as u64)) as u64
    }
    // return whether this memory is allowed for use by the OS (i.e. not reserved | signalled by mem_type being 1)
    pub fn is_usage_allowed(&self) -> bool {
        self.mem_type == 1
    }
}

extern "C" {
    pub static multiboot_info_ptr: *const MultibootInfo;  // TODO: add public api (probably with Mutex)
}
