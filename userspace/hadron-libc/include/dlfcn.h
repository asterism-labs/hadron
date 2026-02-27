/* dlfcn.h — Dynamic linking stubs for Hadron libc.
 *
 * Mesa is configured with -Dshared-glapi=disabled and static driver linking
 * so dlopen/dlsym are not called at runtime.  These stubs satisfy the
 * compiler when Mesa's own configure-time probes include this header.
 */
#ifndef _DLFCN_H
#define _DLFCN_H

#define RTLD_LAZY   0x0001
#define RTLD_NOW    0x0002
#define RTLD_GLOBAL 0x0100
#define RTLD_LOCAL  0x0000
#define RTLD_DEFAULT ((void *)0)

void *dlopen(const char *filename, int flags);
void *dlsym(void *handle, const char *symbol);
int   dlclose(void *handle);
char *dlerror(void);

#endif /* _DLFCN_H */
