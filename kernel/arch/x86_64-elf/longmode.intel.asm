global long_mode_start
extern boot_error

section .text
bits 64
long_mode_start:
    ; zero out all data segment registers
    mov ax, 0
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    ; Initialise kernel stack
    ; (the bootstrapping stack is no longer needed)
    mov esp, kstack_top
    
    ; Hand control over to rust
    extern _kmain
    call _kmain
    ; _kmain should never return

section .bss

; kernel stack
global kstack_top
global kstack_bottom
global kstack_guard_page
align 4096
kstack_guard_page:  ; The guard page is an extra page which is never present, and so will always trigger a page fault
             ; thus ensuring that a stack overflow won't silently corrupt memory
    resb 4096  ; align on a P1 page boundrary
kstack_bottom:
    resb 65536
kstack_top:

; initial kernel heap
; a small amount of space reserved for the kernel heap
; prior to getting a proper allocator for memory set up (which requires processing the ram map and many other things)
align 4
kheap_initial_start:
resb 0x100_0000 ; 16MiB
kheap_initial_end:

section .rodata
; kernel heap initial size
global kheap_initial_addr
global kheap_initial_size
align 4
kheap_initial_addr:
    dd kheap_initial_start
kheap_initial_size:
    dd (kheap_initial_end - kheap_initial_start)
    
; multiboot info ptr
global multiboot_info_ptr
align 8  ; Note: this is allocated as a 64-bit pointer to allow for it to be easily used once we transition to long mode
multiboot_info_ptr:
    resb 8