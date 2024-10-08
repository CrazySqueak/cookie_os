use alloc::vec::Vec;
use core::ptr::addr_of;
use lazy_static::lazy_static;

#[derive(Debug,Clone,Copy)]
#[repr(C,packed)]
struct InfoHeader {
    total_size: u32,
    reserved: u32,
}

extern "C" {
    // start of multiboot info
    static multiboot_info_ptr: *const InfoHeader;
}

#[repr(C,packed)]
union MBTagContentRaw {
    mem_info: (u32,u32),
    mem_map: (u32,u32,MemoryMapEntry),  // the first MemoryMapEntry is a stand in for the start of the list of entries
    rsdp_v1: u8,  // The u8 is a stand in for the actual content
    rsdp_v2: u8,
}

#[derive(Debug,Clone,Copy)]
#[repr(C,packed)]
pub struct MBTagHeader {
    pub tag_type: u32,
    pub tag_size: u32,
}
#[derive(Debug)]
pub struct MBTag {
    pub header: MBTagHeader,
    pub content: MBTagContents,
}
#[derive(Debug)]
pub enum MBTagContents {
    BasicMemInfo {mem_lower: u32, mem_upper: u32},
    MemoryMap {entry_size: u32, entry_version: u32, entries: Vec<MemoryMapEntry>},
    
    // Note: Parsing and validating the ACPI RSDP should be done by a dedicated parser.
    AcpiRsdpV1 { rsdp_virt_addr: usize },  // const pointers are not Sync???
    AcpiRsdpV2 { rsdp_virt_addr: usize },
    
    // Terminates the list of tags
    EndOfTags,
}
impl MBTag {
    // Read a tag from the following pointer, and return a safe
    // representation. An Err containing only the header is returned if the type field is
    // not recognised.
    // SAFETY: There is no guarantee that
    // the pointer is valid. This must be ensured
    // beforehand.
    unsafe fn read_tag(ptr: *const MBTagHeader) -> Result<Self,MBTagHeader> {
        use MBTagContents::*;
        
        let header = *ptr;
        let tag_type = header.tag_type;
        let tag_raw = &*(ptr.add(1) as *const MBTagContentRaw);  // (raw starts after header)
        
        Ok(Self {
            header: header,
            content: match tag_type {
                4 => BasicMemInfo{mem_lower: tag_raw.mem_info.0, mem_upper: tag_raw.mem_info.1},
                
                6 => MemoryMap{entry_size: tag_raw.mem_map.0, entry_version: tag_raw.mem_map.1,
                               entries: {
                                   let entry_size: usize = tag_raw.mem_map.0.try_into().unwrap();
                                   let mut entry_ptr = addr_of!(tag_raw.mem_map.2);
                                   let header_size: u32 = entry_ptr.byte_offset_from(ptr).try_into().unwrap();
                                   let num_entries: usize = (header.tag_size - header_size) as usize / entry_size;
                                   let mut entries = Vec::with_capacity(num_entries);
                                   
                                   // Read entries
                                   for _ in 0..num_entries {
                                       // Read entry
                                       entries.push(*entry_ptr);
                                       // Increment entry ptr
                                       entry_ptr=entry_ptr.byte_add(entry_size);
                                   }
                                   
                                   entries
                }},
                
                14 => AcpiRsdpV1 { rsdp_virt_addr: addr_of!(tag_raw.rsdp_v1) as usize },
                15 => AcpiRsdpV2 { rsdp_virt_addr: addr_of!(tag_raw.rsdp_v2) as usize },
                
                0 => EndOfTags,
                _ => Err(header)?,
            },
        })
    }
}

#[repr(C,packed)]
#[derive(Debug,Clone,Copy)]
pub struct MemoryMapEntry {
    pub base_addr: u64,
    pub length: u64,
    mem_type: u32,
    reserved: u32,
}
impl MemoryMapEntry {
    pub fn is_for_general_use(&self) -> bool {
        // If 1, then this is available for use by the OS
        // If any other number, then it is reserved by the BIOS/ACPI/UEFI/etc.
        self.mem_type == 1
    }
}

lazy_static! {
    pub static ref MULTIBOOT_TAGS: Vec<MBTag> = { unsafe {
        // SAFETY: This requires the multiboot_info_ptr (and the information it points to)
        // to be correctly formatted according to the multiboot2 spec
        // https://www.gnu.org/software/grub/manual/multiboot2/multiboot.html#Boot-information-format
        // Provided that this is true: the values are copied to heap memory,
        // where they are then SAFE for the remainder of the program.
        let info_header = *multiboot_info_ptr;
        let mut tag_ptr = multiboot_info_ptr.add(1) as *const MBTagHeader;
        let tags_end = multiboot_info_ptr.byte_offset(info_header.total_size.try_into().unwrap()) as *const MBTagHeader;
        let mut tags = Vec::new();
        while (tag_ptr as usize) < (tags_end as usize) {
            let tag_size: u32;
            match MBTag::read_tag(tag_ptr){
                Ok(tag_full) => {tag_size=tag_full.header.tag_size; tags.push(tag_full);}
                Err(header) => {tag_size=header.tag_size;}
            };
            // Seek to next tag
            tag_ptr=tag_ptr.byte_add(tag_size.try_into().unwrap());
            // If not eight byte aligned, fix that
            if (tag_ptr as usize)%8 != 0 { tag_ptr=tag_ptr.byte_add(8-((tag_ptr as usize)%8)); }
        };
        tags
    }};
    
    pub static ref MULTIBOOT_MEMORY_MAP: Option<&'static Vec<MemoryMapEntry>> = { for tag in &*MULTIBOOT_TAGS {
        if let MBTagContents::MemoryMap { ref entries, .. } = tag.content { return Some(entries); }
    }; None};
    
    pub static ref ACPI_RSDP_V1_PHYSADDR: Option<usize> = { for tag in &*MULTIBOOT_TAGS {
            if let MBTagContents::AcpiRsdpV1 { rsdp_virt_addr } = tag.content {
                let rsdp_phys_addr = crate::memory::paging::ptaddr_virt_to_phys(rsdp_virt_addr);
                return Some(rsdp_phys_addr);
            }
        }; None };
    pub static ref ACPI_RSDP_V2_PHYSADDR: Option<usize> = { for tag in &*MULTIBOOT_TAGS {
            if let MBTagContents::AcpiRsdpV2 { rsdp_virt_addr } = tag.content {
                let rsdp_phys_addr = crate::memory::paging::ptaddr_virt_to_phys(rsdp_virt_addr);
                return Some(rsdp_phys_addr);
            }
        }; None };
}