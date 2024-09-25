section .text
bits 64

; Note: Switching address spaces is the responsibility of the Rust code as there's a bunch of extra state involved to satisfy RAII instincts
; This simply saves the current state, calling a callback with the stack pointer, and then allows us to load the state (given the appropriate stack pointer).

; extern "sysv64" _cs_push() -> ();
; Save the state, and call the scheduler with the stack pointer as an argument. The scheduler is responsible for saving that stack pointer somewhere, so it can be loaded again when it's time to resume.
; This will return when a corresponding call to _cs_pop(...) is made with the stack pointer as an argument.
; Until then, the computer will be executing other code.
; This is the basic foundation of modern multitasking.
global _cs_push
extern contextswitch_scheduler_cb
extern contextswitch_pop_cb
_cs_push:
    ; Prologue
    ; RIP is saved on the stack for us, and the stack is 8-byte aligned (not 16-byte)
    push RBP  ; Push RBP to the stack (directly above the return addr)
    mov RBP, RSP  ; Set our RBP
    
    ; Save Registers (https://wiki.osdev.org/System_V_ABI#x86-64)
    ; Caller Saved Registers: RAX,RDI,RSI,RDX,RCX,R8,R9,R10,R11 : We don't need to worry about them, as we do not use them (they're already saved by the caller)
    ; Callee Saved Registers: RBX,R12,R13,R14,R15       : We need to save them here.
    ; caller's RSP,RBP,RIP - handled by the prologue/calling convention
    push RBX
    push R12
    push R13
    push R14
    push R15
    ; Save RDI and RSI (to allow cs_new to push values for them). We push RSI here to maintain alignment
    push RDI
    push RSI
    
    ; Save our own RBP as it's also the caller's RSP
    push RBP            ; our base pointer (as if in the function prologue). This is loaded as RSP in the epilogue (prior to returning)
    ; Our RSP is saved by passing it as an argument to the scheduler
    
    ; Then pass stack pointer as argument
    ; The value saved here is 16-byte aligned, with the top two being TOS -> RBP, RIP
    ; Thus meaning that when _cs_pop is called with the value, it can simply do the normal function epilogue and return to us
    ; Keep our first parameter, the command, (RDI) in RDI
    mov RSI, RSP  ; Move the stack pointer into RSI, so it becomes the first argument to the scheduler
    
    ; Call scheduler
    call contextswitch_scheduler_cb
    ; ^ before suspending ^
    ; ========== ==========
    ; V  after  resuming  V
.resume:
    ; Load Registers
    ; our RBP was already loaded by _cs_pop
    pop RSI
    pop RDI
    
    pop R15
    pop R14
    pop R13
    pop R12
    pop RBX
    
    ; Return
    mov RSP, RBP  ; Clear any leftover locals / alignment
    pop RBP  ; Load the previous RBP
    ret      ; Return (thus loading RIP)

; extern "sysv64" _cs_pop(rsp: *const u8, cb_args: *mut T) -> !
; Switch to the given stack frame, and then "return" into it
; The callback is called once the stack has been switched
global _cs_pop
_cs_pop:
    ; We've been passed the stack pointer in RDI
    mov RSP,RDI
    
    ; TOS -> RBP, RIP, [data for _cs_push.resume]
    pop RBP  ; Load the previous RBP
    
    ; call callback
    mov RDI, RSI
    call contextswitch_pop_cb
    ; jump to resume
    jmp _cs_push.resume  ; Resume (effectively a "return" but always to the same place so we don't need to waste stack space)


; extern "sysv64" _cs_newv(entrypoint: extern "sysv64" fn(*mut T) -> !, stack: *const u8, task_args: *mut T) -> *const u8 (rsp)
global _cs_newv
_cs_newv:
    ; prologue
    push RBP
    mov RBP, RSP
    
    ; parameters
    ; rdi = entry point
    ; rsi = new stack
    ; rdx = task first argument
    ; locals
    ; rbp (base pointer) = caller stack
    
    ; switch to new stack
    mov RSP, RSI
    
    ; Initialise stack - this is an analogue of _cs_push, but pushes new values instead of current ones
    push RDI  ; entry point as caller return address
    push RSI  ; stack base as caller's RBP
    mov RSI, RSP  ; save stack pointer - pushed later so _cs_push.resume's RBP -> caller RBP
    push 0 ; RBX
    push 0 ; R12
    push 0 ; R13
    push 0 ; R14
    push 0 ; R15
    push RDX ; RDX becomes task's RDI
    push RCX ; RCX becomes task's RSI (even though we don't intend to accept non-pointer-size task_arg values)
    push RSI  ; stack base as caller's RSP / _cs_push.resume's RBP
    
    ; save new stack's RSP as return value
    mov RAX,RSP
    
    ; epilogue - we don't need to store RSP as a local because we don't clobber RBP, therefore the caller's RSP is kept safe
    mov RSP,RBP
    pop RBP
    ret