/* sched.h — POSIX scheduling API stubs for Hadron */
#ifndef _SCHED_H
#define _SCHED_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Scheduling policies */
#define SCHED_OTHER 0
#define SCHED_FIFO  1
#define SCHED_RR    2

struct sched_param {
    int sched_priority;
};

/* CPU affinity types (used by pthreads for CPU_SET etc.) */
typedef struct {
    unsigned long __bits[16]; /* 1024 CPUs */
} cpu_set_t;

#define CPU_ZERO(setp)         __builtin_memset((setp), 0, sizeof(cpu_set_t))
#define CPU_SET(cpu, setp)     ((setp)->__bits[(cpu)/64] |= (1UL << ((cpu)%64)))
#define CPU_CLR(cpu, setp)     ((setp)->__bits[(cpu)/64] &= ~(1UL << ((cpu)%64)))
#define CPU_ISSET(cpu, setp)   (!!((setp)->__bits[(cpu)/64] & (1UL << ((cpu)%64))))
#define CPU_COUNT(setp)        __cpu_count(setp)

static inline int __cpu_count(const cpu_set_t *s) {
    int n = 0;
    for (int i = 0; i < 16; i++) n += __builtin_popcountl(s->__bits[i]);
    return n;
}

/* Functions */
int sched_yield(void);
int sched_get_priority_max(int policy);
int sched_get_priority_min(int policy);
int sched_getparam(pid_t pid, struct sched_param *param);
int sched_setparam(pid_t pid, const struct sched_param *param);
int sched_getscheduler(pid_t pid);
int sched_setscheduler(pid_t pid, int policy, const struct sched_param *param);

#ifdef __cplusplus
}
#endif

#endif /* _SCHED_H */
