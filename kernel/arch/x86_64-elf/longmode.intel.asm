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
    
    ; print `OKAY` to screen
    ; that's so fucking cool that the tutorial demonstrates our use of RAX
    ; by adding two VGA letters to the "OK" example, I wasn't expecting that /gen
    mov rax, 0x2f592f412f4b2f4f
    mov qword [0xb8000], rax
    hlt