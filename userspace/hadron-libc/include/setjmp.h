/* setjmp.h — Non-local jumps for Hadron libc */
#ifndef _SETJMP_H
#define _SETJMP_H

#include <bits/features.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * jmp_buf layout for x86_64 (8 entries x 8 bytes = 64 bytes):
 *   [0] RBX
 *   [1] RBP
 *   [2] R12
 *   [3] R13
 *   [4] R14
 *   [5] R15
 *   [6] RSP
 *   [7] RIP (return address)
 */
typedef unsigned long jmp_buf[8];
typedef unsigned long sigjmp_buf[8];

int  setjmp(jmp_buf env) __attribute__((returns_twice));
void longjmp(jmp_buf env, int val) __attribute__((noreturn));

int  sigsetjmp(sigjmp_buf env, int savemask) __attribute__((returns_twice));
void siglongjmp(sigjmp_buf env, int val) __attribute__((noreturn));

/* BSD compatibility aliases */
int  _setjmp(jmp_buf env) __attribute__((returns_twice));
void _longjmp(jmp_buf env, int val) __attribute__((noreturn));

#ifdef __cplusplus
}
#endif

#endif /* _SETJMP_H */
