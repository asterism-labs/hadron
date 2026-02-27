/* pthread.h — POSIX threads for Hadron */
#ifndef _PTHREAD_H
#define _PTHREAD_H

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

/* Mutex attributes */
int  pthread_mutexattr_init(pthread_mutexattr_t *attr);
int  pthread_mutexattr_destroy(pthread_mutexattr_t *attr);
int  pthread_mutexattr_settype(pthread_mutexattr_t *attr, int type);

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

#endif /* _PTHREAD_H */
