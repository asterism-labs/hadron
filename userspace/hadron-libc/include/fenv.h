#ifndef _FENV_H
#define _FENV_H

#include <bits/features.h>

#ifdef __cplusplus
extern "C" {
#endif

/* x86_64 floating-point exception flags (mxcsr / x87 status word bits). */
#define FE_INVALID    0x01
#define FE_DENORMAL   0x02
#define FE_DIVBYZERO  0x04
#define FE_OVERFLOW   0x08
#define FE_UNDERFLOW  0x10
#define FE_INEXACT    0x20
#define FE_ALL_EXCEPT (FE_INVALID | FE_DENORMAL | FE_DIVBYZERO | \
                       FE_OVERFLOW | FE_UNDERFLOW | FE_INEXACT)

/* Rounding modes (mxcsr bits 13:14 / x87 CW bits 10:11). */
#define FE_TONEAREST  0x0000
#define FE_DOWNWARD   0x0400
#define FE_UPWARD     0x0800
#define FE_TOWARDZERO 0x0c00

typedef unsigned int fexcept_t;

typedef struct {
    unsigned int __cw;   /* x87 control word */
    unsigned int __sw;   /* x87 status word  */
    unsigned int __mxcsr;
} fenv_t;

#define FE_DFL_ENV ((const fenv_t *) -1)

/* Exception flag access. */
int feclearexcept(int excepts);
int feraiseexcept(int excepts);
int fetestexcept(int excepts);
int fegetexceptflag(fexcept_t *flagp, int excepts);
int fesetexceptflag(const fexcept_t *flagp, int excepts);

/* Rounding mode. */
int fegetround(void);
int fesetround(int round);

/* Environment. */
int fegetenv(fenv_t *envp);
int fesetenv(const fenv_t *envp);
int feholdexcept(fenv_t *envp);
int feupdateenv(const fenv_t *envp);

#ifdef __cplusplus
}
#endif

#endif /* _FENV_H */
