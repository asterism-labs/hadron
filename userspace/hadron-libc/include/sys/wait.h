/* sys/wait.h — Process wait for Hadron libc */
#ifndef _SYS_WAIT_H
#define _SYS_WAIT_H

#include <bits/features.h>

#include <sys/types.h>

/* Wait status macros (Linux-compatible encoding) */
#define WIFEXITED(s)    (((s) & 0x7F) == 0)
#define WEXITSTATUS(s)  (((s) >> 8) & 0xFF)
#define WIFSIGNALED(s)  (((s) & 0x7F) > 0 && ((s) & 0x7F) < 0x7F)
#define WTERMSIG(s)     ((s) & 0x7F)
#define WIFSTOPPED(s)   (((s) & 0xFF) == 0x7F)
#define WSTOPSIG(s)     (((s) >> 8) & 0xFF)

/* Options */
#define WNOHANG   1
#define WUNTRACED 2

pid_t wait(int *status);
pid_t waitpid(pid_t pid, int *status, int options);

#endif /* _SYS_WAIT_H */
