/* The bootloader will look at this image and start execution at the symbol
   designated as the entry point. */
ENTRY(start)

higher_half_offset = 0xFFFF800000000000;
/* Physical addresses so the protected mode boot program can access labels that will be used in the higher half. */
multiboot_info_ptr_P = multiboot_info_ptr - higher_half_offset;
kstack_guard_page_P = kstack_guard_page - higher_half_offset;

/* Tell where the various sections of the object files will be put in the final
   kernel image. */
SECTIONS
{
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
    
    /* LOWER HALF (BOOTSTRAP) */
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
    
    kernel_phys_end = . - higher_half_offset;
}