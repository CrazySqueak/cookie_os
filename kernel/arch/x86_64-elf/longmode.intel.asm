global long_mode_start
extern higher_half_offset

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
    mov rsp, kstack_top
    
    ; Convert multiboot_info_ptr from physical address to virtual address in higher-half
    ; (we couldn't do this in 32-bit code for obvious reasons)
    mov rdx, multiboot_info_ptr
    mov rax, [rdx]
    mov rcx, higher_half_offset
    add rax, rcx
    mov [rdx], rax
    
    ; Note: we no longer un-map vmem as the identity mappings are still necessary for starting up later processors
    ; (the kernel will change the context on boot anyway)
    
    ; Hand control over to rust
    extern _kstart
    call _kstart
    ; _kmain should never return

global long_mode_ap_start
extern next_processor_stack
long_mode_ap_start:
    ; zero out all data segment registers
    mov ax, 0
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    ; Initialise kernel stack
    mov rax, next_processor_stack
    mov qword rsp, [rax]
    ; Set stack to zero, since we're using it - this can also signal to the kernel that we have successfully taken the stack
    mov qword [rax], 0
    
    ; Hand control over to rust
    extern _kstart_ap
    call _kstart_ap
    ; _kapstart should never return

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
    resb 0x10_0000 ; 1MiB - TODO: wait for someone at rust to actually implement the ability to initialise huge structures on the heap instead of the stack ;65536
kstack_top:

; initial kernel heap
; a small amount of space reserved for the kernel heap
; prior to getting a proper allocator for memory set up (which requires processing the ram map and many other things)
align 4
kheap_initial_start:
resb 0x80_0000 ; 8MiB - we can expand it as needed
kheap_initial_end:

; multiboot info ptr
global multiboot_info_ptr
align 8
multiboot_info_ptr:
    resb 8

section .data
; number of processors initialised (u16)
; note: this starts at one as the BSP code does not contain an INC instruction
global processors_started
align 2
processors_started:
    dw 1

section .rodata
; kernel heap initial size
global kheap_initial_addr
global kheap_initial_size
align 8
kheap_initial_addr:
    dq kheap_initial_start
kheap_initial_size:
    dq (kheap_initial_end - kheap_initial_start)
