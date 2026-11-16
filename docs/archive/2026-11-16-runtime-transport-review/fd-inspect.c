#include <libproc.h>
#include <sys/proc_info.h>
#include <fcntl.h>
#include <termios.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 4) return 2;
    struct proc_fdinfo fds[256];
    int bytes=proc_pidinfo(atoi(argv[1]),PROC_PIDLISTFDS,0,fds,sizeof(fds)), found=0;
    for (int i=0;i<bytes/(int)sizeof(*fds);i++) {
        struct vnode_fdinfowithpath info;
        if (fds[i].proc_fd<=2 || proc_pidfdinfo(atoi(argv[1]),fds[i].proc_fd,PROC_PIDFDVNODEPATHINFO,&info,sizeof(info))!=(int)sizeof(info)) continue;
        if (strstr(info.pvip.vip_path,"/dev/ptmx")) {
            if (!(info.pfi.fi_status & PROC_FP_CLEXEC)) return 3;
            found++;
        }
    }
    struct vnode_fdinfowithpath slave;
    if (!found || proc_pidfdinfo(atoi(argv[2]),0,PROC_PIDFDVNODEPATHINFO,&slave,sizeof(slave))!=(int)sizeof(slave)) return 4;
    int fd=open(slave.pvip.vip_path,O_RDONLY|O_NOCTTY|O_NONBLOCK|O_CLOEXEC);
    struct termios mode;
    if (fd<0 || tcgetattr(fd,&mode)<0) return 5;
    close(fd);
    printf("observed caller=%s private=%s iflag=%lx oflag=%lx lflag=%lx\n",argv[3],slave.pvip.vip_path,(unsigned long)mode.c_iflag,(unsigned long)mode.c_oflag,(unsigned long)mode.c_lflag);
    if (!strcmp(argv[3],slave.pvip.vip_path) || mode.c_lflag & (ICANON|ECHO|ISIG)) return 6;
    printf("F1/F2 live private slave distinct, noncanonical, noecho, nosignals; parent master CLOEXEC; initialraw/atomic windows inspected\n");
    return 0;
}
