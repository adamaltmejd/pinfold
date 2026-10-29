#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

static int failure(int error, const char *operation) {
    if (error == ENOSPC) {
        puts("cache-space-exhausted");
        return 1;
    }
    errno = error;
    perror(operation);
    return 2;
}

int main(void) {
    static const unsigned char zeros[1024 * 1024];
    uint64_t remaining = UINT64_C(8) * 1024 * 1024 * 1024;
    int fd;
    do {
        fd = open("/capacity/data", O_WRONLY | O_CREAT | O_TRUNC, 0600);
    } while (fd < 0 && errno == EINTR);
    if (fd < 0)
        return failure(errno, "open cache fixture");
    while (remaining) {
        size_t length = remaining < sizeof(zeros) ? (size_t)remaining : sizeof(zeros);
        ssize_t written = write(fd, zeros, length);
        if (written > 0) {
            remaining -= (uint64_t)written;
            continue;
        }
        if (written < 0 && errno == EINTR)
            continue;
        int error = written < 0 ? errno : EIO;
        close(fd);
        return failure(error, "write cache fixture");
    }
    int synced;
    do {
        synced = fsync(fd);
    } while (synced < 0 && errno == EINTR);
    if (synced < 0) {
        int error = errno;
        close(fd);
        return failure(error, "sync cache fixture");
    }
    if (close(fd) < 0)
        return failure(errno, "close cache fixture");
    fputs("cache fixture did not exhaust its bounded writes\n", stderr);
    return 3;
}
