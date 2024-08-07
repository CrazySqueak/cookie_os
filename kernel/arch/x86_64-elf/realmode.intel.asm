
section .text
bits 16  ; real mode :-(
global ap_trampoline_realmode
align 4096
ap_trampoline_realmode:
    cli
    cld
    ; load code segment 0
    jmp 0:.part2
.part2:
    ; load data segment 0
    mov ax,0
    mov ds,ax
    ; load gdt
    lgdt [trampoline_gdt_ptr]
    mov eax, cr0  ; load cr0
    or eax, 1 ; enable protected mode
    mov cr0, eax ; save cr0
    ; perform long-jump to enter 32-bit code
    ; note: segment numbers are offsets (bytes), not indicies
    jmp 8:ap_protected_mode_bridge

bits 32
extern ap_start
ap_protected_mode_bridge:
    jmp ap_start  ; ap_start is 2MB inwards in memory. good luck getting there with only 16-bit code

section .data
trampoline_gdt:
    ; Null Segment - #0
    dq 0x00000000_00000000
    ; Code Segment 32 - #1
    ; 00 = base31..24, Cf = flags+limit19..16, 9a = access byte, 00 = base39..32
    ; 0000 = base31..16, FFFF = limit15..0
    dq 0x00Cf9a00_0000FFFF
    ; Data Segment 32 - #2
    ; 00 = base31..24, 9f = flags+limit19..16, 92 = access byte, 00 = base39..32
    ; 0000 = base31..16, FFFF = limit15..0
    dq 0x00Cf9200_0000FFFF
    ; Task State Segment - #3
    ; https://wiki.osdev.org/Symmetric_Multiprocessing#AP_Initialization_Code
    dq 0x00CF8900_00000068
trampoline_gdt_ptr:
    dw trampoline_gdt_ptr - trampoline_gdt - 1
    dw trampoline_gdt
    dw 0, 0