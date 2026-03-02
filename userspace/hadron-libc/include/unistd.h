/* unistd.h — POSIX API for Hadron libc */
#ifndef _UNISTD_H
#define _UNISTD_H

#include <bits/features.h>
#include <stddef.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

/* POSIX version constants */
#define _POSIX_VERSION  200809L
#define _POSIX2_VERSION 200809L
#define _XOPEN_VERSION  700

/* sysconf() name constants (subset) */
#define _SC_PAGE_SIZE        30
#define _SC_PAGESIZE         _SC_PAGE_SIZE
#define _SC_NPROCESSORS_ONLN 84
#define _SC_NPROCESSORS_CONF 83
#define _SC_PHYS_PAGES       85
#define _SC_CLK_TCK           2
#define _SC_OPEN_MAX          5
#define _SC_ARG_MAX           0

/* access() mode bits */
#define F_OK 0
#define R_OK 4
#define W_OK 2
#define X_OK 1

/* ---- Core file I/O (POSIX.1-1990) ----------------------------------------- */

ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
int     close(int fd);
off_t   lseek(int fd, off_t offset, int whence);
int     dup(int fd);
int     dup2(int oldfd, int newfd);
int     pipe(int fds[2]);
int     isatty(int fd);
int     unlink(const char *path);
int     rmdir(const char *path);
int     access(const char *path, int mode);

/* ---- Process (POSIX.1-1990) ------------------------------------------------ */

pid_t getpid(void);
pid_t getppid(void);
pid_t fork(void);
pid_t vfork(void);
void  _exit(int status) __attribute__((noreturn));

/* ---- Working directory (POSIX.1-1990) -------------------------------------- */

char *getcwd(char *buf, size_t size);
int   chdir(const char *path);
int   fchdir(int fd);

/* ---- Directory / FS (POSIX.1-1990) ---------------------------------------- */

int mkdir(const char *path, unsigned int mode);
int rename(const char *oldpath, const char *newpath);

/* ---- User/group IDs (POSIX.1-1990) ---------------------------------------- */

uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);

/* ---- Process groups / sessions (POSIX.1-1990) ------------------------------ */

int   setpgid(pid_t pid, pid_t pgid);
pid_t getpgid(pid_t pid);
pid_t setsid(void);

/* ---- Sleep (POSIX.1-1990) -------------------------------------------------- */

unsigned int sleep(unsigned int seconds);
int          usleep(unsigned int usec);

/* ---- POSIX.1-2001 additions ------------------------------------------------ */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
int  pipe2(int fds[2], int flags);
int  dup3(int oldfd, int newfd, int flags);
long sysconf(int name);
int  getpagesize(void);
int  fsync(int fd);
int  fdatasync(int fd);
int  truncate(const char *path, off_t length);
int  ftruncate(int fd, off_t length);
ssize_t pread(int fd, void *buf, size_t count, off_t offset);
ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset);
int  symlink(const char *target, const char *linkpath);
ssize_t readlink(const char *path, char *buf, size_t bufsiz);
int  link(const char *oldpath, const char *newpath);
int  chown(const char *path, uid_t owner, gid_t group);
int  fchown(int fd, uid_t owner, gid_t group);
int  chmod(const char *path, unsigned int mode);
int  fchmod(int fd, unsigned int mode);
#endif

/* ---- exec family ----------------------------------------------------------- */

#if defined(_HADRON_POSIX_2001) || defined(_HADRON_DEFAULT)
int execv(const char *path, char *const argv[]);
int execve(const char *path, char *const argv[], char *const envp[]);
int execvp(const char *file, char *const argv[]);
int execl(const char *path, const char *arg, ...);
int execlp(const char *file, const char *arg, ...);
int execle(const char *path, const char *arg, ...);
#endif

/* ---- syscall --------------------------------------------------------------- */

#ifdef _HADRON_GNU_EXTENSIONS
long syscall(long number, ...);
#endif

/* ---- GNU extensions -------------------------------------------------------- */

#ifdef _HADRON_GNU_EXTENSIONS
int  getopt(int argc, char *const argv[], const char *optstring);
extern int   optind, opterr, optopt;
extern char *optarg;
ssize_t copy_file_range(int fd_in, long long *off_in,
                        int fd_out, long long *off_out,
                        size_t len, unsigned int flags);
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* _UNISTD_H */
