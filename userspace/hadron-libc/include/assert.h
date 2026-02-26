/* assert.h — Assertions for Hadron libc */
#ifndef _ASSERT_H
#define _ASSERT_H

#ifdef NDEBUG
#define assert(expr) ((void)0)
#else
#define assert(expr)                                                      \
    ((expr) ? (void)0                                                     \
            : (fprintf(stderr, "Assertion failed: %s, file %s, line %d\n", \
                       #expr, __FILE__, __LINE__),                        \
               abort()))
#endif

/* C11 static_assert */
#ifndef static_assert
#define static_assert _Static_assert
#endif

#endif /* _ASSERT_H */
