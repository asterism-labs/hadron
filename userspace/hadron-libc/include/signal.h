/* signal.h — Signals for Hadron libc */
#ifndef _SIGNAL_H
#define _SIGNAL_H

#include <bits/features.h>

#include <sys/types.h>
#include <stdint.h>

#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)
#define SIG_ERR ((void (*)(int))-1)

/* Signal numbers (Linux-compatible) */
#define SIGHUP     1
#define SIGINT     2
#define SIGQUIT    3
#define SIGILL     4
#define SIGTRAP    5
#define SIGABRT    6
#define SIGBUS     7
#define SIGFPE     8
#define SIGKILL    9
#define SIGUSR1   10
#define SIGSEGV   11
#define SIGUSR2   12
#define SIGPIPE   13
#define SIGALRM   14
#define SIGTERM   15
#define SIGCHLD   17
#define SIGCONT   18
#define SIGSTOP   19
#define SIGTSTP   20
#define SIGTTIN   21
#define SIGTTOU   22
#define SIGWINCH  28

/* Real-time signal range (Linux) */
#define SIGRTMIN  34
#define SIGRTMAX  64

/* Alternate signal stack size */
#define SIGSTKSZ  8192
#define MINSIGSTKSZ 2048

/* sigprocmask how values */
#define SIG_BLOCK   0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

/* sig_atomic_t: type that can be read/written atomically in signal handlers */
typedef int sig_atomic_t;

typedef unsigned long sigset_t;

typedef void (*sighandler_t)(int);

/* sigaction flags */
#define SA_NOCLDSTOP  0x00000001
#define SA_NOCLDWAIT  0x00000002
#define SA_SIGINFO    0x00000004
#define SA_ONSTACK    0x08000000
#define SA_RESTART    0x10000000
#define SA_NODEFER    0x40000000
#define SA_RESETHAND  0x80000000

/* siginfo_t (minimal) */
typedef struct {
    int      si_signo;
    int      si_errno;
    int      si_code;
    pid_t    si_pid;
    uid_t    si_uid;
    int      si_status;
    void    *si_addr;
    long     si_value;
    long     _padding[8];
} siginfo_t;

/* struct sigaction */
struct sigaction {
    union {
        void (*sa_handler)(int);
        void (*sa_sigaction)(int, siginfo_t *, void *);
    };
    sigset_t sa_mask;
    int      sa_flags;
    void   (*sa_restorer)(void);
};

/* Alternate signal stack flags */
#define SS_ONSTACK 1
#define SS_DISABLE 2

/* Alternate signal stack (stack_t) */
typedef struct {
    void  *ss_sp;
    int    ss_flags;
    size_t ss_size;
} stack_t;

#ifdef __cplusplus
extern "C" {
#endif

sighandler_t signal(int signum, sighandler_t handler);
int  sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
int  sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
int  raise(int sig);
int  kill(pid_t pid, int sig);
int  sigemptyset(sigset_t *set);
int  sigfillset(sigset_t *set);
int  sigaddset(sigset_t *set, int signum);
int  sigdelset(sigset_t *set, int signum);
int  sigismember(const sigset_t *set, int signum);
int  sigaltstack(const stack_t *ss, stack_t *old_ss);

#ifdef __cplusplus
}
#endif

#endif /* _SIGNAL_H */
