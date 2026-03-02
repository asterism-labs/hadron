/* pthread.h — POSIX threads for Hadron */
#ifndef _PTHREAD_H
#define _PTHREAD_H

#include <bits/features.h>

#include <stdint.h>
#include <stddef.h>

/* ---- Types ---------------------------------------------------------------- */

typedef uint64_t pthread_t;

/* pthread_attr_t: opaque, 64 bytes (stack_size at offset 0) */
typedef struct {
    size_t stack_size;
    unsigned char _opaque[56];
} pthread_attr_t;

/* pthread_mutex_t: 40 bytes, compatible with glibc size.
 * Lock word at offset 0 (u32); rest is padding. */
typedef struct {
    volatile uint32_t __lock;
    unsigned char _pad[36];
} pthread_mutex_t;

#define PTHREAD_MUTEX_INITIALIZER { 0, {0} }

typedef struct {
    int __kind;
} pthread_mutexattr_t;

/* pthread_cond_t: sequence counter + waiter count */
typedef struct {
    volatile uint32_t __seq;
    volatile uint32_t __waiters;
} pthread_cond_t;

#define PTHREAD_COND_INITIALIZER { 0, 0 }

typedef struct {
    int _unused;
} pthread_condattr_t;

/* pthread_once_t: 0=uninitialized, 1=in-progress, 2=done */
typedef volatile uint32_t pthread_once_t;

#define PTHREAD_ONCE_INIT 0

typedef uint32_t pthread_key_t;

/* pthread_rwlock_t: read-write lock (64 bytes) */
typedef struct {
    volatile uint32_t __state;
    volatile uint32_t __waiters;
    unsigned char _pad[56];
} pthread_rwlock_t;

typedef struct {
    int _unused;
} pthread_rwlockattr_t;

#define PTHREAD_RWLOCK_INITIALIZER { 0, 0, {0} }

/* pthread_barrier_t: barrier synchronization */
typedef struct {
    volatile uint32_t __count;
    volatile uint32_t __waiting;
    volatile uint32_t __seq;
    unsigned char _pad[52];
} pthread_barrier_t;

typedef struct {
    unsigned int __count;
} pthread_barrierattr_t;

#define PTHREAD_BARRIER_SERIAL_THREAD (-1)

/* pthread_spinlock_t */
typedef volatile uint32_t pthread_spinlock_t;

/* ---- Error codes ---------------------------------------------------------- */
#ifndef EBUSY
# define EBUSY    16
#endif
#ifndef ETIMEDOUT
# define ETIMEDOUT 110
#endif
#ifndef EDEADLK
# define EDEADLK  35
#endif
#ifndef EAGAIN
# define EAGAIN   11
#endif

/* ---- Function declarations ------------------------------------------------ */

/* Thread lifecycle */
int  pthread_create(pthread_t *thread, const pthread_attr_t *attr,
                    void *(*start_routine)(void *), void *arg);
int  pthread_join(pthread_t thread, void **retval);
pthread_t pthread_self(void);
void pthread_exit(void *retval);
int  pthread_detach(pthread_t thread);
int  pthread_equal(pthread_t t1, pthread_t t2);

/* Thread attributes */
int  pthread_attr_init(pthread_attr_t *attr);
int  pthread_attr_destroy(pthread_attr_t *attr);
int  pthread_attr_setstacksize(pthread_attr_t *attr, size_t stacksize);
int  pthread_attr_getstacksize(const pthread_attr_t *attr, size_t *stacksize);

/* Mutex */
int  pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attr);
int  pthread_mutex_destroy(pthread_mutex_t *mutex);
int  pthread_mutex_lock(pthread_mutex_t *mutex);
int  pthread_mutex_trylock(pthread_mutex_t *mutex);
int  pthread_mutex_unlock(pthread_mutex_t *mutex);
#include <time.h>
int  pthread_mutex_timedlock(pthread_mutex_t *mutex, const struct timespec *abstime);

/* Mutex attributes */
int  pthread_mutexattr_init(pthread_mutexattr_t *attr);
int  pthread_mutexattr_destroy(pthread_mutexattr_t *attr);
int  pthread_mutexattr_settype(pthread_mutexattr_t *attr, int type);
int  pthread_mutexattr_gettype(const pthread_mutexattr_t *attr, int *type);
#ifdef _HADRON_GNU_EXTENSIONS
int  pthread_mutexattr_setrobust(pthread_mutexattr_t *attr, int robustness);
int  pthread_mutexattr_getrobust(const pthread_mutexattr_t *attr, int *robustness);
#endif

/* Process-fork handler */
#if defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
int  pthread_atfork(void (*prepare)(void), void (*parent)(void), void (*child)(void));
#endif

/* Condition variables */
int  pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr);
int  pthread_cond_destroy(pthread_cond_t *cond);
int  pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex);
int  pthread_cond_signal(pthread_cond_t *cond);
int  pthread_cond_broadcast(pthread_cond_t *cond);

/* Once */
int  pthread_once(pthread_once_t *once_control, void (*init_routine)(void));

/* Thread-specific data */
int  pthread_key_create(pthread_key_t *key, void (*destructor)(void *));
int  pthread_key_delete(pthread_key_t key);
void *pthread_getspecific(pthread_key_t key);
int  pthread_setspecific(pthread_key_t key, const void *value);

/* Scheduling */
int  sched_yield(void);
int  pthread_attr_setdetachstate(pthread_attr_t *attr, int detachstate);
int  pthread_attr_getdetachstate(const pthread_attr_t *attr, int *detachstate);

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1

/* Read-write locks */
int  pthread_rwlock_init(pthread_rwlock_t *rwlock, const pthread_rwlockattr_t *attr);
int  pthread_rwlock_destroy(pthread_rwlock_t *rwlock);
int  pthread_rwlock_rdlock(pthread_rwlock_t *rwlock);
int  pthread_rwlock_tryrdlock(pthread_rwlock_t *rwlock);
int  pthread_rwlock_wrlock(pthread_rwlock_t *rwlock);
int  pthread_rwlock_trywrlock(pthread_rwlock_t *rwlock);
int  pthread_rwlock_unlock(pthread_rwlock_t *rwlock);

/* Barriers */
int  pthread_barrier_init(pthread_barrier_t *barrier,
                          const pthread_barrierattr_t *attr, unsigned count);
int  pthread_barrier_destroy(pthread_barrier_t *barrier);
int  pthread_barrier_wait(pthread_barrier_t *barrier);

/* Spinlocks */
int  pthread_spin_init(pthread_spinlock_t *lock, int pshared);
int  pthread_spin_destroy(pthread_spinlock_t *lock);
int  pthread_spin_lock(pthread_spinlock_t *lock);
int  pthread_spin_trylock(pthread_spinlock_t *lock);
int  pthread_spin_unlock(pthread_spinlock_t *lock);

/* Condition variable attributes with clock */
int  pthread_condattr_setclock(pthread_condattr_t *attr, int clock_id);
int  pthread_condattr_getclock(const pthread_condattr_t *attr, int *clock_id);
int  pthread_condattr_init(pthread_condattr_t *attr);
int  pthread_condattr_destroy(pthread_condattr_t *attr);

/* Timed wait */
#include <time.h>
int  pthread_cond_timedwait(pthread_cond_t *cond, pthread_mutex_t *mutex,
                            const struct timespec *abstime);

/* Signal delivery to thread */
int  pthread_kill(pthread_t thread, int sig);
int  pthread_sigmask(int how, const void *set, void *oldset);

/* Robust mutexes */
#define PTHREAD_MUTEX_ROBUST  1
#define PTHREAD_MUTEX_STALLED 0

/* Process-sharing attributes */
#define PTHREAD_PROCESS_PRIVATE 0
#define PTHREAD_PROCESS_SHARED  1

int  pthread_mutexattr_setpshared(pthread_mutexattr_t *attr, int pshared);
int  pthread_mutexattr_getpshared(const pthread_mutexattr_t *attr, int *pshared);

/* Cancellation */
int  pthread_cancel(pthread_t thread);
int  pthread_setcancelstate(int state, int *oldstate);
int  pthread_setcanceltype(int type, int *oldtype);
void pthread_testcancel(void);

#define PTHREAD_CANCEL_ENABLE  0
#define PTHREAD_CANCEL_DISABLE 1
#define PTHREAD_CANCEL_DEFERRED    0
#define PTHREAD_CANCEL_ASYNCHRONOUS 1
#define PTHREAD_CANCELED ((void *)-1)

/* Cleanup handlers (implemented as macros per POSIX) */
struct __pthread_cleanup_frame {
    void (*__routine)(void *);
    void *__arg;
    int  __do_it;
};

#define pthread_cleanup_push(routine, arg) \
    do { struct __pthread_cleanup_frame __frame = { (routine), (arg), 1 };

#define pthread_cleanup_pop(execute) \
    __frame.__do_it = (execute); \
    if (__frame.__do_it) __frame.__routine(__frame.__arg); } while (0)

#endif /* _PTHREAD_H */
