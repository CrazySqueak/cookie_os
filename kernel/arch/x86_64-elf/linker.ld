/* The bootloader will look at this image and start execution at the symbol
   designated as the entry point. */
ENTRY(start)

hh_data_offset = 0xFFFF800000000000;
higher_half_offset = 0xFFFFFFFF80000000;
/* Physical addresses so the protected mode boot program can access labels that will be used in the higher half. */
multiboot_info_ptr_P = multiboot_info_ptr - higher_half_offset;
processors_started_P = processors_started - higher_half_offset;
/* The kernel stack is located in hhdata) */
kstack_guard_page_P = kstack_guard_page - hh_data_offset;

/* Tell where the various sections of the object files will be put in the final
   kernel image. */
SECTIONS
{
    /* LOW LOW HALF (REAL MODE TRAMPOLINE) */
    . = 0x8000;
    
    .trampoline.text BLOCK(4K) : ALIGN(4K) {
        */realmode.o(.text)
    }
    .trampoline.data BLOCK(4K) : ALIGN(4K) {
        */realmode.o(.data)
    }
    
    /* LOWER HALF (BOOTSTRAP) */
	/* It used to be universally recommended to use 1M as a start offset,
	   as it was effectively guaranteed to be available under BIOS systems.
	   However, UEFI has made things more complicated, and experimental data
	   strongly suggests that 2M is a safer place to load. In 2016, a new
	   feature was introduced to the multiboot2 spec to inform bootloaders
	   that a kernel can be loaded anywhere within a range of addresses and
	   will be able to relocate itself to run from such a loader-selected
	   address, in order to give the loader freedom in selecting a span of
	   memory which is verified to be available by the firmware, in order to
	   work around this issue. This does not use that feature, so 2M was
	   chosen as a safer option than the traditional 1M. */
	. = 2M;
    
    kernel_phys_start = .;
    
    /* Remember to give these sections different names than the higher half as otherwise you will get 100 confusing error messages that give no explanation. hours_wasted += 3. noise_complaints_from_neighbours += 1. */
	/* First put the multiboot header, as it is required to be put very early
	   in the image or the bootloader won't recognize the file format.
	   Next we'll put the .text section. */
	.boot.text BLOCK(4K) : ALIGN(4K)
	{
		*(.multiboot_header)
		*/boot.o(.text)
	}
	/* Read-only data. */
	.boot.rodata BLOCK(4K) : ALIGN(4K)
	{
		*/boot.o(.rodata)
	}
	/* Read-write data (initialized) */
	.boot.data BLOCK(4K) : ALIGN(4K)
	{
		*/boot.o(.data)
	}
	/* Read-write data (uninitialized) and stack */
	.boot.bss BLOCK(4K) : ALIGN(4K)
	{
		*/boot.o(COMMON)
		*/boot.o(.bss)
	}
    
    /* LOWER HIGHER HALF (KERNEL STACK/HEAP) */
    . = ALIGN(4K);
    kdata_phys_start = .;
    . += hh_data_offset;
    
    .kheap ALIGN(4K) : AT(ADDR (.kheap) - hh_data_offset)
    {
        *(.kheap)
    }
    
    . = ALIGN(4K);
    . -= hh_data_offset;
    kdata_phys_end = .;
    
    /* HIGHER HALF (KERNEL) */
    . += higher_half_offset;
    
	.text ALIGN(4K) : AT(ADDR (.text) - higher_half_offset)
	{
		*(.text*)
	}
	.rodata ALIGN(4K) : AT(ADDR (.rodata) - higher_half_offset)
	{
		*(.rodata*)
	}
	.data ALIGN(4K) : AT(ADDR (.data) - higher_half_offset)
	{
		*(.data*)
	}
	.bss ALIGN(4K) : AT(ADDR (.bss) - higher_half_offset)
	{
		*(COMMON)
		*(.bss*)
	}
    
    /* unwind tables */
    .eh_frame ALIGN(4K) : AT(ADDR (.eh_frame) - higher_half_offset)
    {
        *(.eh_frame)
    }
    .eh_frame_hdr ALIGN(4K) : AT(ADDR (.eh_frame_hdr) - higher_half_offset)
    {
        *(.eh_frame_hdr)
    }
    __eh_frame_hdr_start = ADDR(.eh_frame_hdr);
    __eh_frame_hdr_end = ADDR(.eh_frame_hdr) + SIZEOF(.eh_frame_hdr);
    __eh_frame_start = ADDR(.eh_frame);
    __eh_frame_end = ADDR(.eh_frame) + SIZEOF(.eh_frame);
    
    kernel_phys_end = . - higher_half_offset;
    
}