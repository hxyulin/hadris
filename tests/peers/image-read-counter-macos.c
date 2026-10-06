#include <sys/stat.h>
#include <sys/types.h>
#include <sys/uio.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <stdint.h>
static struct stat target;
static int active;
static _Atomic uint64_t calls, requested, delivered, seeks, failures;
__attribute__((constructor)) static void setup(void) {
    const char *path = getenv("HADRIS_IO_IMAGE");
    active = path && !stat(path, &target);
}
static int image_fd(int fd) {
    struct stat st;
    return active && !fstat(fd, &st) && st.st_dev == target.st_dev && st.st_ino == target.st_ino;
}
static void note(size_t size, ssize_t result) {
    atomic_fetch_add(&calls, 1); atomic_fetch_add(&requested, size);
    if (result < 0) atomic_fetch_add(&failures, 1);
    else atomic_fetch_add(&delivered, (uint64_t)result);
}
static ssize_t count_read(int fd, void *buf, size_t size) {
    int matched = image_fd(fd); ssize_t result = read(fd, buf, size);
    if (matched) note(size, result); return result;
}
static ssize_t count_pread(int fd, void *buf, size_t size, off_t offset) {
    int matched = image_fd(fd); ssize_t result = pread(fd, buf, size, offset);
    if (matched) note(size, result); return result;
}
static ssize_t count_readv(int fd, const struct iovec *iov, int n) {
    int matched = image_fd(fd); ssize_t result = readv(fd, iov, n);
    if (matched) {
        size_t size=0;
        if(result>=0) for(int i=0;i<n;i++) size+=iov[i].iov_len;
        note(size,result);
    }
    return result;
}
static off_t count_lseek(int fd, off_t offset, int whence) {
    int matched = image_fd(fd); off_t result = lseek(fd, offset, whence);
    if (matched) { atomic_fetch_add(&seeks,1); if(result<0)atomic_fetch_add(&failures,1); } return result;
}
__attribute__((destructor)) static void report(void) {
    fprintf(stderr,"IMAGE_IO,%llu,%llu,%llu,%llu,%llu\n",(unsigned long long)calls,(unsigned long long)requested,(unsigned long long)delivered,(unsigned long long)seeks,(unsigned long long)failures);
}
#define INTERPOSE(replacement, original) \
__attribute__((used)) static struct { const void *new_fn; const void *old_fn; } interpose_##original \
__attribute__((section("__DATA,__interpose"))) = { (const void *)(uintptr_t)&replacement, (const void *)(uintptr_t)&original }
INTERPOSE(count_read,read);
INTERPOSE(count_pread,pread);
INTERPOSE(count_readv,readv);
INTERPOSE(count_lseek,lseek);
