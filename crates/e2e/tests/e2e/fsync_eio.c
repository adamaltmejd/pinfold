#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#if defined(__x86_64__)
#define HOST_ARCH AUDIT_ARCH_X86_64
#elif defined(__aarch64__)
#define HOST_ARCH AUDIT_ARCH_AARCH64
#else
#error Unsupported host architecture
#endif

static volatile sig_atomic_t stopped;
static void stop(int signal_number) { stopped = signal_number; }
static int64_t milliseconds(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now)) return -1;
    return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int send_listener(int socket, int listener) {
    union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int))]; } control = {0};
    char byte = 'L';
    struct iovec iov = { &byte, 1 };
    struct msghdr message = { .msg_iov = &iov, .msg_iovlen = 1,
        .msg_control = control.bytes, .msg_controllen = sizeof(control.bytes) };
    struct cmsghdr *header = CMSG_FIRSTHDR(&message);
    header->cmsg_level = SOL_SOCKET;
    header->cmsg_type = SCM_RIGHTS;
    header->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(header), &listener, sizeof(listener));
    return sendmsg(socket, &message, MSG_NOSIGNAL) == 1 ? 0 : -1;
}

static int receive_listener(int socket) {
    union { struct cmsghdr align; char bytes[CMSG_SPACE(sizeof(int))]; } control = {0};
    char byte;
    struct iovec iov = { &byte, 1 };
    struct msghdr message = { .msg_iov = &iov, .msg_iovlen = 1,
        .msg_control = control.bytes, .msg_controllen = sizeof(control.bytes) };
    if (recvmsg(socket, &message, MSG_CMSG_CLOEXEC) != 1) return -1;
    struct cmsghdr *header = CMSG_FIRSTHDR(&message);
    if (byte != 'L' || message.msg_flags & (MSG_TRUNC | MSG_CTRUNC) || !header ||
        header->cmsg_level != SOL_SOCKET || header->cmsg_type != SCM_RIGHTS ||
        header->cmsg_len != CMSG_LEN(sizeof(int)) || CMSG_NXTHDR(&message, header)) {
        errno = EPROTO; return -1;
    }
    int listener;
    memcpy(&listener, CMSG_DATA(header), sizeof(listener));
    return listener;
}

static void child(int socket, char **command) {
    struct sock_filter instructions[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, HOST_ARCH, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_fsync, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
    };
    struct sock_fprog program = { sizeof(instructions) / sizeof(instructions[0]), instructions };
    if (setpgid(0, 0) || prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)) _exit(125);
    int listener = syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
        SECCOMP_FILTER_FLAG_NEW_LISTENER, &program);
    if (listener < 0 || send_listener(socket, listener)) _exit(125);
    close(listener);
    close(socket);
    execvp(command[0], command);
    _exit(127);
}

/* Subreaping also catches descendants that changed their process group. */
static int reap(pid_t child_pid, int *child_status) {
    kill(-child_pid, SIGKILL);
    kill(child_pid, SIGKILL);
    for (;;) {
        char path[80];
        snprintf(path, sizeof(path), "/proc/self/task/%ld/children", (long)getpid());
        FILE *children = fopen(path, "re");
        if (!children) return -1;
        long pid;
        while (fscanf(children, "%ld", &pid) == 1) kill((pid_t)pid, SIGKILL);
        int read_error = ferror(children);
        fclose(children);
        if (read_error) return -1;
        int status;
        pid_t found = waitpid(-1, &status, 0);
        if (found == child_pid) *child_status = status;
        else if (found < 0 && errno == ECHILD) return 0;
        else if (found < 0 && errno != EINTR) return -1;
    }
}

int main(int argc, char **argv) {
    if (argc < 5 || strcmp(argv[3], "--")) {
        fprintf(stderr, "usage: %s IMAGES_DIRECTORY NEW_RECEIPT -- CLI [ARG...]\n", argv[0]);
        return 125;
    }
    int receipt = open(argv[2], O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (receipt < 0) { perror("receipt"); return 125; }
    int directory = -1, listener = -1, pidfd = -1, sockets[2] = {-1, -1};
    pid_t child_pid = -1;
    int status = -1, injections = 0, stale = 0, failure = 0;
    uint64_t notification = 0, argument = 0;
    uint32_t tid = 0;
    struct stat target = {0}, before = {0}, after = {0};
    struct seccomp_notif *request = NULL;
    struct seccomp_notif_resp *response = NULL;
    const char *stage = "setup";
#define REQUIRE(condition) do { if (!(condition)) { failure = errno ? errno : EIO; goto done; } } while (0)
    directory = open(argv[1], O_PATH | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    REQUIRE(directory >= 0 && fstat(directory, &target) == 0);
    REQUIRE(target.st_uid == geteuid() && !(target.st_mode & 077));
    if (fstatat(directory, "catalog.json", &before, AT_SYMLINK_NOFOLLOW)) {
        REQUIRE(errno == ENOENT);
        memset(&before, 0, sizeof(before));
    } else REQUIRE(S_ISREG(before.st_mode));
    struct seccomp_notif_sizes sizes;
    REQUIRE(syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0, &sizes) == 0);
    REQUIRE(sizes.seccomp_notif >= sizeof(*request) && sizes.seccomp_notif_resp >= sizeof(*response));
    request = calloc(1, sizes.seccomp_notif);
    response = calloc(1, sizes.seccomp_notif_resp);
    REQUIRE(request && response && prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0);
    struct sigaction action = { .sa_handler = stop };
    sigemptyset(&action.sa_mask);
    REQUIRE(sigaction(SIGINT, &action, NULL) == 0 && sigaction(SIGTERM, &action, NULL) == 0);
    REQUIRE(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets) == 0);
    int64_t deadline = milliseconds();
    REQUIRE(deadline >= 0);
    deadline += 300000;
    child_pid = fork();
    REQUIRE(child_pid >= 0);
    if (!child_pid) { close(sockets[0]); child(sockets[1], &argv[4]); }
    close(sockets[1]); sockets[1] = -1;
    pidfd = syscall(SYS_pidfd_open, child_pid, 0);
    REQUIRE(pidfd >= 0);
    stage = "supervise";
    for (;;) {
        int64_t now = milliseconds();
        if (stopped || now < 0 || now >= deadline) { failure = stopped ? EINTR : ETIMEDOUT; break; }
        struct pollfd events[] = {
            { listener < 0 ? sockets[0] : listener, POLLIN, 0 }, { pidfd, POLLIN, 0 }
        };
        int ready = poll(events, 2, (int)(deadline - now));
        if (ready < 0 && errno == EINTR) continue;
        REQUIRE(ready >= 0);
        if (events[1].revents & POLLIN) break;
        if (!ready) { failure = ETIMEDOUT; break; }
        REQUIRE(events[0].revents & POLLIN);
        if (listener < 0) {
            listener = receive_listener(sockets[0]);
            REQUIRE(listener >= 0);
            close(sockets[0]); sockets[0] = -1;
            continue;
        }
        memset(request, 0, sizes.seccomp_notif);
        if (ioctl(listener, SECCOMP_IOCTL_NOTIF_RECV, request)) {
            if (errno == ENOENT || errno == EINTR) { stale++; continue; }
            REQUIRE(0);
        }
        REQUIRE(request->data.nr == SYS_fsync && request->data.arch == HOST_ARCH);
        char path[96];
        snprintf(path, sizeof(path), "/proc/%u/fd/%llu", request->pid,
            (unsigned long long)request->data.args[0]);
        struct stat observed;
        int inject = 0;
        if (!injections && stat(path, &observed) == 0 && observed.st_dev == target.st_dev &&
            observed.st_ino == target.st_ino && S_ISDIR(observed.st_mode)) {
            if (fstatat(directory, "catalog.json", &after, AT_SYMLINK_NOFOLLOW) == 0) {
                REQUIRE(S_ISREG(after.st_mode));
                inject = after.st_dev != before.st_dev || after.st_ino != before.st_ino;
            } else REQUIRE(errno == ENOENT);
        }
        if (ioctl(listener, SECCOMP_IOCTL_NOTIF_ID_VALID, &request->id)) {
            if (errno == ENOENT) { stale++; continue; }
            REQUIRE(0);
        }
        memset(response, 0, sizes.seccomp_notif_resp);
        response->id = request->id;
        response->error = inject ? -EIO : 0;
        response->flags = inject ? 0 : SECCOMP_USER_NOTIF_FLAG_CONTINUE;
        if (ioctl(listener, SECCOMP_IOCTL_NOTIF_SEND, response)) {
            if (errno == ENOENT) { stale++; continue; }
            REQUIRE(0);
        }
        if (inject) {
            injections++; notification = request->id; tid = request->pid;
            argument = request->data.args[0];
        }
    }
done:
    if (child_pid > 0 && reap(child_pid, &status) && !failure) failure = errno ? errno : EIO;
    if (listener >= 0) close(listener);
    if (pidfd >= 0) close(pidfd);
    for (int index = 0; index < 2; index++) if (sockets[index] >= 0) close(sockets[index]);
    int child_exit = status >= 0 && WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    int child_signal = status >= 0 && WIFSIGNALED(status) ? WTERMSIG(status) : 0;
    int written = dprintf(receipt, "{\"stage\":\"%s\",\"supervisor_errno\":%d,\"injections\":%d,\"injected_errno\":%d,"
        "\"stale_notifications\":%d,\"notification\":%llu,\"tid\":%u,\"fd\":%llu,"
        "\"directory_device\":%llu,\"directory_inode\":%llu,\"before_catalog_inode\":%llu,"
        "\"after_catalog_inode\":%llu,\"child_exit\":%d,\"child_signal\":%d}\n",
        stage, failure, injections, injections ? EIO : 0, stale, (unsigned long long)notification, tid,
        (unsigned long long)argument, (unsigned long long)target.st_dev,
        (unsigned long long)target.st_ino, (unsigned long long)before.st_ino,
        (unsigned long long)after.st_ino, child_exit, child_signal);
    int receipt_error = written < 0 || fsync(receipt) != 0;
    close(receipt);
    if (directory >= 0) close(directory);
    free(request); free(response);
    return failure || receipt_error || injections != 1 ? 125 : (child_exit >= 0 ? child_exit : 128 + child_signal);
}
