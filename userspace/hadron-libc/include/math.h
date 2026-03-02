/* math.h — Mathematical functions for Hadron libc.
 *
 * Mesa uses these for floating-point rounding, trig, and exp/log ops.
 * Implementations are provided by compiler-rt builtins; declarations here.
 */
#ifndef _MATH_H
#define _MATH_H

#include <bits/features.h>

#define M_E        2.7182818284590452354
#define M_LOG2E    1.4426950408889634074
#define M_LOG10E   0.43429448190325182765
#define M_LN2      0.69314718055994530942
#define M_LN10     2.30258509299404568402
#define M_PI       3.14159265358979323846
#define M_PI_2     1.57079632679489661923
#define M_PI_4     0.78539816339744830962
#define M_1_PI     0.31830988618379067154
#define M_2_PI     0.63661977236758134308
#define M_2_SQRTPI 1.12837916709551257390
#define M_SQRT2    1.41421356237309504880
#define M_SQRT1_2  0.70710678118654752440

#define HUGE_VAL   __builtin_huge_val()
#define HUGE_VALF  __builtin_huge_valf()
#define INFINITY   __builtin_inff()
#define NAN        __builtin_nanf("")

#define FP_NAN       0
#define FP_INFINITE  1
#define FP_ZERO      2
#define FP_SUBNORMAL 3
#define FP_NORMAL    4

/* ilogb special return values (per C99: FP_ILOGB0=INT_MIN, FP_ILOGBNAN=INT_MAX) */
#define FP_ILOGB0   (-2147483647 - 1)
#define FP_ILOGBNAN  2147483647

#define fpclassify(x)  __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, (x))
#define isnan(x)       __builtin_isnan(x)
#define isinf(x)       __builtin_isinf(x)
#define isfinite(x)    __builtin_isfinite(x)
#define isnormal(x)    __builtin_isnormal(x)
#define signbit(x)     __builtin_signbit(x)

#ifdef __cplusplus
extern "C" {
#endif

/* Basic arithmetic */
double      fabs(double x);
float       fabsf(float x);
long double fabsl(long double x);
double      sqrt(double x);
float       sqrtf(float x);
long double sqrtl(long double x);
double      cbrt(double x);
float       cbrtf(float x);
long double cbrtl(long double x);
double      fma(double x, double y, double z);
float       fmaf(float x, float y, float z);
long double fmal(long double x, long double y, long double z);

/* Rounding */
double      floor(double x);
float       floorf(float x);
long double floorl(long double x);
double      ceil(double x);
float       ceilf(float x);
long double ceill(long double x);
double      round(double x);
float       roundf(float x);
long double roundl(long double x);
double      trunc(double x);
float       truncf(float x);
long double truncl(long double x);
double      rint(double x);
float       rintf(float x);
long double rintl(long double x);
long        lrint(double x);
long        lrintf(float x);
long        lrintl(long double x);
long long   llrint(double x);
long long   llrintf(float x);
long long   llrintl(long double x);
long        lround(double x);
long        lroundf(float x);
long        lroundl(long double x);
long long   llround(double x);
long long   llroundf(float x);
long long   llroundl(long double x);
double      nearbyint(double x);
float       nearbyintf(float x);
long double nearbyintl(long double x);

/* Power / exponential */
double      pow(double base, double exp);
float       powf(float base, float exp);
long double powl(long double base, long double exp);
double      exp(double x);
float       expf(float x);
long double expl(long double x);
double      exp2(double x);
float       exp2f(float x);
long double exp2l(long double x);
double      expm1(double x);
float       expm1f(float x);
long double expm1l(long double x);
double      log(double x);
float       logf(float x);
long double logl(long double x);
double      log2(double x);
float       log2f(float x);
long double log2l(long double x);
double      log10(double x);
float       log10f(float x);
long double log10l(long double x);
double      log1p(double x);
float       log1pf(float x);
long double log1pl(long double x);
double      logb(double x);
float       logbf(float x);
long double logbl(long double x);
int         ilogb(double x);
int         ilogbf(float x);
int         ilogbl(long double x);

/* Trigonometric */
double      sin(double x);
float       sinf(float x);
long double sinl(long double x);
double      cos(double x);
float       cosf(float x);
long double cosl(long double x);
double      tan(double x);
float       tanf(float x);
long double tanl(long double x);
double      asin(double x);
float       asinf(float x);
long double asinl(long double x);
double      acos(double x);
float       acosf(float x);
long double acosl(long double x);
double      atan(double x);
float       atanf(float x);
long double atanl(long double x);
double      atan2(double y, double x);
float       atan2f(float y, float x);
long double atan2l(long double y, long double x);
void        sincos(double x, double *s, double *c);
void        sincosf(float x, float *s, float *c);

/* Hyperbolic */
double      sinh(double x);
float       sinhf(float x);
long double sinhl(long double x);
double      cosh(double x);
float       coshf(float x);
long double coshl(long double x);
double      tanh(double x);
float       tanhf(float x);
long double tanhl(long double x);
double      asinh(double x);
float       asinhf(float x);
long double asinhl(long double x);
double      acosh(double x);
float       acoshf(float x);
long double acoshl(long double x);
double      atanh(double x);
float       atanhf(float x);
long double atanhl(long double x);

/* Min/max */
double      fmin(double a, double b);
float       fminf(float a, float b);
long double fminl(long double a, long double b);
double      fmax(double a, double b);
float       fmaxf(float a, float b);
long double fmaxl(long double a, long double b);
double      fdim(double a, double b);
float       fdimf(float a, float b);
long double fdiml(long double a, long double b);

/* Remainder */
double      fmod(double x, double y);
float       fmodf(float x, float y);
long double fmodl(long double x, long double y);
double      remainder(double x, double y);
float       remainderf(float x, float y);
long double remainderl(long double x, long double y);
double      remquo(double x, double y, int *quo);
float       remquof(float x, float y, int *quo);
long double remquol(long double x, long double y, int *quo);
double      modf(double x, double *iptr);
float       modff(float x, float *iptr);
long double modfl(long double x, long double *iptr);

/* Next representable value */
double      nextafter(double x, double y);
float       nextafterf(float x, float y);
long double nextafterl(long double x, long double y);
double      nexttoward(double x, long double y);
float       nexttowardf(float x, long double y);
long double nexttowardl(long double x, long double y);

/* BSD aliases for remainder and scalbn */
#if defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_BSD)
double      drem(double x, double y);
float       dremf(float x, float y);
double      scalb(double x, double y);
float       scalbf(float x, float y);
#endif

/* Frexp / ldexp / scalbn */
double      frexp(double x, int *exp);
float       frexpf(float x, int *exp);
long double frexpl(long double x, int *exp);
double      ldexp(double x, int exp);
float       ldexpf(float x, int exp);
long double ldexpl(long double x, int exp);
double      scalbn(double x, int n);
float       scalbnf(float x, int n);
long double scalbnl(long double x, int n);
double      scalbln(double x, long n);
float       scalblnf(float x, long n);
long double scalblnl(long double x, long n);

/* Copy sign / sign manipulation */
double      copysign(double x, double y);
float       copysignf(float x, float y);
long double copysignl(long double x, long double y);

/* Misc */
double      hypot(double x, double y);
float       hypotf(float x, float y);
long double hypotl(long double x, long double y);
double      erf(double x);
float       erff(float x);
long double erfl(long double x);
double      erfc(double x);
float       erfcf(float x);
long double erfcl(long double x);
double      lgamma(double x);
float       lgammaf(float x);
long double lgammal(long double x);
double      tgamma(double x);
float       tgammaf(float x);
long double tgammal(long double x);
double      j0(double x);
double      j1(double x);
double      jn(int n, double x);
float       j0f(float x);
float       j1f(float x);
float       jnf(int n, float x);
double      y0(double x);
double      y1(double x);
double      yn(int n, double x);
float       y0f(float x);
float       y1f(float x);
float       ynf(int n, float x);

/* Reentrant lgamma (glibc extension) */
#if defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_DEFAULT)
double      lgamma_r(double x, int *signp);
float       lgammaf_r(float x, int *signp);
long double lgammal_r(long double x, int *signp);
#endif

/* GNU math extensions: exp10, pow10 */
#ifdef _HADRON_GNU_EXTENSIONS
double      exp10(double x);
float       exp10f(float x);
long double exp10l(long double x);
double      pow10(double x);
float       pow10f(float x);
long double pow10l(long double x);
#endif

/* sincosl (GNU/glibc extension) */
#if defined(_HADRON_GNU_EXTENSIONS) || defined(_HADRON_DEFAULT)
void sincosl(long double x, long double *s, long double *c);
#endif

/* signgam: sign of last lgamma() result (BSD/XSI extension) */
#ifdef _HADRON_GNU_EXTENSIONS
extern int signgam;
#endif

/* Comparison macros (C99 — implemented as builtins) */
#define isgreater(x, y)      __builtin_isgreater(x, y)
#define isgreaterequal(x, y) __builtin_isgreaterequal(x, y)
#define isless(x, y)         __builtin_isless(x, y)
#define islessequal(x, y)    __builtin_islessequal(x, y)
#define islessgreater(x, y)  __builtin_islessgreater(x, y)
#define isunordered(x, y)    __builtin_isunordered(x, y)

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _MATH_H */
