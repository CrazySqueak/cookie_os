section .text
bits 64

; Note: Switching address spaces is the responsibility of the Rust code as there's a bunch of extra state involved to satisfy RAII instincts
; This simply saves the current state, calling a callback with the stack pointer, and then allows us to load the state (given the appropriate stack pointer).

; extern "sysv64" _cs_push(scheduler: extern "sysv64" fn(rsp: *const u8)->!) -> ();
; Save the state, and call the scheduler with the stack pointer as an argument. The scheduler is responsible for saving that stack pointer somewhere, so it can be loaded again when it's time to resume.
; This will return when a corresponding call to _cs_pop(...) is made with the stack pointer as an argument.
; Until then, the computer will be executing other code.
; This is the basic foundation of modern multitasking.
global _cs_push
_cs_push:
    ; Prologue
    ; RIP is saved on the stack for us, and the stack is 8-byte aligned (not 16-byte)
    push RBP  ; Push RBP to the stack (directly above the return addr)
    mov RBP, RSP  ; Set our RBP
    
    ; Save Registers (https://wiki.osdev.org/System_V_ABI#x86-64)
    ; Caller Saved Registers: RAX,RDI,RSI,RDX,RCX,R8,R9,R10,R11 : We don't need to worry about them, as we do not use them (they're already saved by the caller)
    ; Callee Saved Registers: RBX,R12,R13,R14,R15       : We need to save them here.
    ; RSP,RBP,RIP - handled by the prologue/calling convention
    push RBX
    push R12
    push R13
    push R14
    push R15
    
    ; Push "return address" and RBP for when _cs_pop is called
    ; This is done as if we had "called" _cs_pop ourselves, so the return epilogue works correctly
    ; Note: stack is 16-byte aligned after this push V - also we don't need to push .resume as it's always the same label, so jmp is used instead of ret.
    push RBP            ; our base pointer (as if in the function prologue). This is loaded as RSP in the epilogue (prior to returning)
    ; We don't need to preserve prologue:RBP=RSP (besides as the argument to the scheduler) as _cs_pop doesn't use RBP
    
    ; Then pass stack pointer as argument
    ; The value saved here is 16-byte aligned, with the top two being TOS -> RBP, RIP
    ; Thus meaning that when _cs_pop is called with the value, it can simply do the normal function epilogue and return to us
    mov RSI, RDI  ; Move our first argument (scheduler fn pointer) into RSI
    mov RDI, RSP  ; Move the stack pointer into RDI, so it becomes the first argument to the scheduler
    
    ; Call scheduler
    call RSI
    ; ^ before suspending ^
    ; ========== ==========
    ; V  after  resuming  V
.resume:
    ; Load Registers
    pop R15
    pop R14
    pop R13
    pop R12
    pop RBX
    
    ; Return
    mov RSP, RBP  ; Clear any leftover locals / alignment
    pop RBP  ; Load the previous RBP
    ret      ; Return (thus loading RIP)

; extern "sysv64" _cs_pop(rsp: *const u8) -> !
; Switch to the given stack frame, and then "return" into it
global _cs_pop
_cs_pop:
    ; We've been passed the stack pointer in RDI
    mov RSP,RDI
    
    ; TOS -> RBP, RIP, [data for _cs_push.resume]
    pop RBP  ; Load the previous RBP
    jmp _cs_push.resume  ; Resume (effectively a "return" but always to the same place so we don't need to waste stack space)
