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

#define fpclassify(x)  __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, (x))
#define isnan(x)       __builtin_isnan(x)
#define isinf(x)       __builtin_isinf(x)
#define isfinite(x)    __builtin_isfinite(x)
#define isnormal(x)    __builtin_isnormal(x)
#define signbit(x)     __builtin_signbit(x)

/* Basic arithmetic */
double fabs(double x);
float  fabsf(float x);
double sqrt(double x);
float  sqrtf(float x);
double cbrt(double x);
double fma(double x, double y, double z);
float  fmaf(float x, float y, float z);

/* Rounding */
double floor(double x);
float  floorf(float x);
double ceil(double x);
float  ceilf(float x);
double round(double x);
float  roundf(float x);
double trunc(double x);
float  truncf(float x);
double rint(double x);
float  rintf(float x);
long   lrint(double x);
long   lrintf(float x);

/* Power / exponential */
double pow(double base, double exp);
float  powf(float base, float exp);
double exp(double x);
float  expf(float x);
double exp2(double x);
float  exp2f(float x);
double log(double x);
float  logf(float x);
double log2(double x);
float  log2f(float x);
double log10(double x);
float  log10f(float x);

/* Trigonometric */
double sin(double x);
float  sinf(float x);
double cos(double x);
float  cosf(float x);
double tan(double x);
float  tanf(float x);
double asin(double x);
float  asinf(float x);
double acos(double x);
float  acosf(float x);
double atan(double x);
float  atanf(float x);
double atan2(double y, double x);
float  atan2f(float y, float x);
void   sincos(double x, double *s, double *c);
void   sincosf(float x, float *s, float *c);

/* Hyperbolic */
double sinh(double x);
float  sinhf(float x);
double cosh(double x);
float  coshf(float x);
double tanh(double x);
float  tanhf(float x);

/* Min/max/clamp */
double fmin(double a, double b);
float  fminf(float a, float b);
double fmax(double a, double b);
float  fmaxf(float a, float b);

/* Remainder */
double fmod(double x, double y);
float  fmodf(float x, float y);
double modf(double x, double *iptr);
float  modff(float x, float *iptr);

/* Frexp / ldexp */
double frexp(double x, int *exp);
float  frexpf(float x, int *exp);
double ldexp(double x, int exp);
float  ldexpf(float x, int exp);

/* Misc */
double hypot(double x, double y);
float  hypotf(float x, float y);
double scalbn(double x, int n);
float  scalbnf(float x, int n);

#endif /* _MATH_H */
