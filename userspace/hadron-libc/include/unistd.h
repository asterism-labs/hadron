/* unistd.h — POSIX API for Hadron libc */
#ifndef _UNISTD_H
#define _UNISTD_H

#include <stddef.h>
#include <sys/types.h>

#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

/* File I/O */
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int     close(int fd);
off_t   lseek(int fd, off_t offset, int whence);
int     dup(int fd);
int     dup2(int oldfd, int newfd);
int     pipe(int fds[2]);
int     pipe2(int fds[2], int flags);
int     isatty(int fd);
int     unlink(const char *path);

/* Process */
pid_t getpid(void);
pid_t getppid(void);
pid_t fork(void);
pid_t vfork(void);
void _exit(int status) __attribute__((noreturn));

/* Working directory */
char *getcwd(char *buf, size_t size);
int   chdir(const char *path);
int   mkdir(const char *path, unsigned int mode);

/* Process groups and sessions */
int   setpgid(pid_t pid, pid_t pgid);
pid_t getpgid(pid_t pid);
pid_t setsid(void);

/* Sleep */
unsigned int sleep(unsigned int seconds);
int usleep(unsigned int usec);

#endif /* _UNISTD_H */
