/* stddef.h — Standard definitions for Hadron libc */
#ifndef _STDDEF_H
#define _STDDEF_H

#ifndef NULL
#define NULL ((void *)0)
#endif

typedef unsigned long size_t;
typedef long          ptrdiff_t;

#ifndef __cplusplus
typedef int           wchar_t;
#endif

#define offsetof(type, member) __builtin_offsetof(type, member)

#endif /* _STDDEF_H */
