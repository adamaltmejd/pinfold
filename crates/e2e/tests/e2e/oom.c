#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static void fail(const char *operation) {
    fprintf(stderr, "failure=%s errno=%d\n", operation, errno);
    exit(1);
}

static int score(const char *path) {
    FILE *file = fopen(path, "r");
    int value;
    if (!file) fail("score-open");
    if (fscanf(file, "%d", &value) != 1) fail("score-read");
    if (fclose(file)) fail("score-close");
    return value;
}

static void expect_score(int expected) {
    int actual = score("/proc/self/oom_score_adj");
    printf("pid=%ld score=%d expected=%d\n", (long)getpid(), actual, expected);
    if (actual != expected) { errno = 0; fail("score-mismatch"); }
}

static void write_score(const char *api, const char *value, int denied) {
    char path[80];
    snprintf(path, sizeof(path), "/proc/self/%s", api);
    int fd = open(path, O_WRONLY | O_CLOEXEC);
    if (fd < 0) fail("write-open");
    errno = 0;
    ssize_t count = write(fd, value, strlen(value));
    int saved = errno;
    if (close(fd)) fail("write-close");
    printf("pid=%ld api=%s value=%s count=%ld errno=%d denied=%d\n",
           (long)getpid(), api, value, (long)count, saved, denied);
    if (denied ? !(count == -1 && saved == EACCES)
               : count != (ssize_t)strlen(value)) {
        errno = saved;
        fail("write-result");
    }
}

static void controls(void) {
    write_score("oom_score_adj", "-999", 0);
    expect_score(-999);
    write_score("oom_score_adj", "-1000", 1);
    expect_score(-999);
    write_score("oom_adj", "15", 0);
    expect_score(1000);
    write_score("oom_score_adj", "-999", 0);
    expect_score(-999);
    write_score("oom_adj", "-17", 1);
    expect_score(-999);
}

static void parent_score(int expected) {
    char path[80];
    snprintf(path, sizeof(path), "/proc/%ld/oom_score_adj", (long)getppid());
    int actual = score(path);
    printf("parent_pid=%ld parent_score=%d expected=%d\n",
           (long)getppid(), actual, expected);
    if (actual != expected) { errno = 0; fail("parent-score-mismatch"); }
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc != 2) { errno = EINVAL; fail("mode"); }
    parent_score(!strcmp(argv[1], "descendant") ? -999 : -1000);
    if (!strcmp(argv[1], "probe")) {
        controls();
        pid_t child = fork();
        if (child < 0) fail("fork");
        if (child == 0) {
            execl(argv[0], argv[0], "descendant", (char *)NULL);
            fail("exec-descendant");
        }
        int status;
        if (waitpid(child, &status, 0) != child) fail("wait-descendant");
        if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
            errno = 0;
            fail("descendant-status");
        }
    } else if (!strcmp(argv[1], "descendant")) {
        expect_score(-999);
        controls();
    } else if (!strcmp(argv[1], "hog")) {
        write_score("oom_score_adj", "-999", 0);
        expect_score(-999);
        const size_t bytes = (size_t)1 << 30;
        long page = sysconf(_SC_PAGESIZE);
        if (page <= 0) fail("page-size");
        // Skip upfront commit accounting; page touches create physical pressure.
        volatile unsigned char *memory = mmap(NULL, bytes, PROT_READ | PROT_WRITE,
                                             MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
        if (memory == MAP_FAILED) fail("mmap");
        printf("pressure_bytes=%zu\n", bytes);
        for (size_t offset = 0; offset < bytes; offset += (size_t)page)
            memory[offset] = 1;
        errno = 0;
        fail("pressure-survived");
    } else {
        errno = EINVAL;
        fail("mode");
    }
    return 0;
}
