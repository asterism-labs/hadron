# Minimal test init binary for Hadron OS.
# Calls SYS_DEBUG_LOG to print a message, then SYS_TASK_EXIT.

.text
.globl _start
_start:
    # SYS_DEBUG_LOG(buf, len)
    movq $0xF1, %rax         # syscall number = SYS_DEBUG_LOG
    leaq msg(%rip), %rdi     # arg0 = pointer to message
    movq $22, %rsi            # arg1 = length
    syscall

    # SYS_TASK_EXIT(0)
    movq $0x00, %rax         # syscall number = SYS_TASK_EXIT
    xorq %rdi, %rdi          # arg0 = status = 0
    syscall

    # Should never reach here.
    ud2

.section .rodata
msg:
    .ascii "Hello from userspace!\n"
