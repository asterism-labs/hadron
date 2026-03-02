/* semaphore.h — POSIX semaphores for Hadron libc */
#ifndef _SEMAPHORE_H
#define _SEMAPHORE_H

#include <bits/features.h>
#include <stddef.h>
#include <time.h>

#define SEM_FAILED ((sem_t *)-1)
#define SEM_VALUE_MAX 2147483647

/* Opaque semaphore type (8 bytes: value + waiter count) */
typedef struct {
    volatile int __value;
    volatile int __waiters;
} sem_t;

#ifdef __cplusplus
extern "C" {
#endif

int  sem_init(sem_t *sem, int pshared, unsigned int value);
int  sem_destroy(sem_t *sem);
int  sem_post(sem_t *sem);
int  sem_wait(sem_t *sem);
int  sem_trywait(sem_t *sem);
int  sem_getvalue(sem_t *sem, int *sval);

int  sem_timedwait(sem_t *sem, const struct timespec *abs_timeout);

sem_t *sem_open(const char *name, int oflag, ...);
int    sem_close(sem_t *sem);
int    sem_unlink(const char *name);

#ifdef __cplusplus
}
#endif

#endif /* _SEMAPHORE_H */
