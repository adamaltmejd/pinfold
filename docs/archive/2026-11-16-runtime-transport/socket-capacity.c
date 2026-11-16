#include <stdio.h>
#include <termios.h>
#include <sys/ioctl.h>
#include <sys/un.h>
int main(void) {
    printf("sockaddr_un.sun_path=%zu bytes including terminator\n",
           sizeof(((struct sockaddr_un *)0)->sun_path));
#ifdef __APPLE__
    printf("Darwin TIOCSETA/W/F=%lu,%lu,%lu\n", (unsigned long)TIOCSETA,
           (unsigned long)TIOCSETAW, (unsigned long)TIOCSETAF);
#endif
}
