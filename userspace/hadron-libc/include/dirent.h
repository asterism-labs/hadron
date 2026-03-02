/* dirent.h — Directory traversal for Hadron libc */
#ifndef _DIRENT_H
#define _DIRENT_H

#include <bits/features.h>

#include <stdint.h>

#define NAME_MAX 255

struct dirent {
    uint64_t d_ino;
    uint8_t  d_type;
    char     d_name[NAME_MAX + 1];
};

/* File types */
#define DT_UNKNOWN 0
#define DT_FIFO    1
#define DT_CHR     2
#define DT_DIR     4
#define DT_BLK     6
#define DT_REG     8
#define DT_LNK    10
#define DT_SOCK   12

/* Opaque directory stream */
typedef struct _DIR DIR;

DIR           *opendir(const char *name);
struct dirent *readdir(DIR *dirp);
int            closedir(DIR *dirp);

#endif /* _DIRENT_H */
