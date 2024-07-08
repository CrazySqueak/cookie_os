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
extern long_mode_start

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
    
    ; Enable Paging and enter Long Mode
    call configure_identity_paging
    call enable_paging_and_long_mode
    
    ; load the 64-bit GDT
    lgdt [gdt64.pointer]
    ; And far jump to start
    jmp gdt64.kcode:long_mode_start
    
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

; Paging
%define PAGEFLAG_PRESENT_WRITEABLE      0b0000_0011
%define PAGEFLAG_PRESENT_WRITEABLE_HUGE 0b1000_0011
%define PAGETABLE_NUM_ENTRIES 512
configure_identity_paging:
    ; P4 = Page-Map Level-4 Table (address bits 47-39, each entry covers 512GiB of virtual address space)
    ; P3 = Page-Directory Pointer Table (address bits 38-30, each entry covers 1GiB of virtual address space)
    ; P2 = Page-Directory Table (address bits 29-21, each entry covers 2MiB of virtual address space)
    ; P1 = Page Table (address bits 20-12, each entry covers 4KiB of virtual address space)
    ; map first P4 entry (0x0000_0000... - 0x0000_0080...)
    mov eax, p3_table
    or eax, PAGEFLAG_PRESENT_WRITEABLE
    mov [p4_table], eax
    
    ; map first P3 entry (0x...0000_0000 - 0x...4000_0000)
    mov eax, p2_table
    or eax, PAGEFLAG_PRESENT_WRITEABLE
    mov [p3_table], eax
    
    ; map each P2 entry to a huge page (a huge page covers the entire address space for that entry, instead of pointing to another table containing subdivisions)
    mov ecx, 0
    .map_p2:
    mov eax, 0x20_0000 ;2MiB
    mul ecx ; multiplied by ecx gives us our start address of the ecx-th page in P2 (when identity paging, assuming this is the first P2)
    or eax, PAGEFLAG_PRESENT_WRITEABLE_HUGE
    mov [p2_table + ecx * 8], eax ; place in map (indexed by counter)
    
    inc ecx
    cmp ecx, PAGETABLE_NUM_ENTRIES
    jne .map_p2  ; keep going
    ret  ; else return

; Enable Paging + Long Mode
%define CR4FLAG_PAE 1<<5
%define MSR_EFER 0xC0000080
%define EFERFLAG_LONGMODE 1<<8
%define CR0FLAG_PAGING 1<<31
enable_paging_and_long_mode:
    ; load P4 into CR3
    mov eax, p4_table
    mov cr3, eax
    
    ; enable PAE in cr4
    mov eax, cr4
    or eax, CR4FLAG_PAE
    mov cr4, eax
    
    ; set long mode in the EFER MSR
    mov ecx, MSR_EFER
    rdmsr
    or eax, EFERFLAG_LONGMODE
    wrmsr
    
    ; Enable paging
    mov eax, cr0
    or eax, CR0FLAG_PAGING
    mov cr0, eax
    
    ret

; Error handling
; Prints `ERR: ` and the given error message to screen and hangs.
; parameter: error code in eax, which is an index into our table of error codes
global boot_error
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
; global descriptor table
gdt64:
    dq 0 ; null entry
.kcode: equ $ - gdt64 ; pointer to entry #1
    dq (1<<43) | (1<<44) | (1<<47) | (1<<53) ; entry #1 - kernel code (64-bit)
.pointer:
    dw $ - gdt64 - 1
    dq gdt64

section .bss
; identity paging
align 4096
p4_table:
    resb 4096
p3_table:
    resb 4096
p2_table:
    resb 4096
global p4_table
global p3_table
global p2_table
global bstack_bottom
global bstack_top
; stack
align 16
bstack_bottom:
    resb 65536
bstack_top: