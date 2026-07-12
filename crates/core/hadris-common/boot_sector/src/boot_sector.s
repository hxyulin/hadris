.section .boot, "awx"
.code16
.global _start
.extern message

_start:
    # Disable interrupts
    cli

    # Clear the segment registers
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    # Clear the direction flag
    cld

    # Set the stack pointer
    # The stack grows downwards
    mov sp, 0x7C00

    # Enable interrupts
    sti

    # Print the message
    call print_message

hang:
    cli
    hlt
    jmp hang

print_message:
    mov si, message
    mov bx, 0x0000
    mov ah, 0x0E

.print_loop:
    lodsb
    cmp al, 0
    jz .done
    int 0x10
    jmp .print_loop
.done:
    ret
