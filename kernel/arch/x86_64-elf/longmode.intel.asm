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
    
    ; Hand control over to rust
    extern _kmain
    call _kmain