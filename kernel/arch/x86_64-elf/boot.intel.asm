section .multiboot_header
align 4
header_start:
    dd 0xe85250d6                ; magic number (multiboot 2)
    dd 0                         ; architecture 0 (protected mode i386)
    dd header_end - header_start ; header length
    ; checksum
    dd 0x100000000 - (0xe85250d6 + 0 + (header_end - header_start))

    ; insert optional multiboot tags here

    ; required end tag
    dw 0    ; type
    dw 0    ; flags
    dd 8    ; size
header_end:

global start

; big thanks to https://os.phil-opp.com/entering-longmode/
section .text
bits 32 ; we are started in protected mode
start:
    ; Initialise stack
    mov esp, bstack_top
    
    ; Checks
    call check_multiboot
    call check_cpuid
    call check_long_mode
    
    ; ok
    mov dword [0xb8000], 0x2f4b2f4f
    hlt
    
; CHECKS

check_multiboot:
    cmp eax, 0x36d76289
    jne .no_multiboot
    ret
.no_multiboot:
    mov eax, 1
    jmp boot_error

check_cpuid:
    ; Check if CPUID is supported by attempting to flip the ID bit (bit 21)
    ; in the FLAGS register. If we can flip it, CPUID is available.

    ; Copy FLAGS in to EAX via stack
    pushfd
    pop eax

    ; Copy to ECX as well for comparing later on
    mov ecx, eax

    ; Flip the ID bit
    xor eax, 1 << 21

    ; Copy EAX to FLAGS via the stack
    push eax
    popfd

    ; Copy FLAGS back to EAX (with the flipped bit if CPUID is supported)
    pushfd
    pop eax

    ; Restore FLAGS from the old version stored in ECX (i.e. flipping the
    ; ID bit back if it was ever flipped).
    push ecx
    popfd

    ; Compare EAX and ECX. If they are equal then that means the bit
    ; wasn't flipped, and CPUID isn't supported.
    cmp eax, ecx
    je .no_cpuid
    ret
.no_cpuid:
    mov eax, 2
    jmp boot_error

check_long_mode:
    ; test if extended processor info in available
    mov eax, 0x80000000    ; implicit argument for cpuid
    cpuid                  ; get highest supported argument
    cmp eax, 0x80000001    ; it needs to be at least 0x80000001
    jb .no_long_mode       ; if it's less, the CPU is too old for long mode

    ; use extended info to test if long mode is available
    mov eax, 0x80000001    ; argument for extended processor info
    cpuid                  ; returns various feature bits in ecx and edx
    test edx, 1 << 29      ; test if the LM-bit is set in the D-register
    jz .no_long_mode       ; If it's not set, there is no long mode
    ret
.no_long_mode:
    mov eax, 3
    jmp boot_error

; Error handling
; Prints `ERR: ` and the given error message to screen and hangs.
; parameter: error code in eax, which is an index into our table of error codes
boot_error:
    mov dword [0xb8000], 0x4f524f45
    mov dword [0xb8004], 0x4f3a4f52
    mov word  [0xb8008], 0x4f20
    
    mov dword eax, [.err_table + eax*4]
    mov dword ebx, 0xb800a
    .displp:
    mov byte dl, [eax]
    mov byte [ebx], dl
    mov byte [ebx+1], 0x4f
    add eax, 1
    add ebx, 2
    cmp byte [eax], 0
    jne .displp
    
    hlt

section .rodata
; error table
.err_table:
dd .err_unknown ; 0 - unknown
dd .err_no_multiboot ; 1 - no_multiboot
dd .err_no_cpuid ; 2 - no_cpuid
dd .err_8086 ; 3 - long mode unsupported
.err_unknown:
db 'UNKNOWN_ERR',0
.err_no_multiboot:
db 'NO_MULTIBOOT',0
.err_no_cpuid:
db 'NO_CPUID',0
.err_8086:
db 'NO_THIS_IS_AN_INTEL_8086',0

section .bss
; stack
bstack_bottom:
    resb 16384
bstack_top: