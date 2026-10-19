#define _GNU_SOURCE
#include <sys/xattr.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <errno.h>
#include <unistd.h>
struct E{uint16_t tag,perm;uint32_t id;};
struct A{uint32_t version;struct E e[5];};
int main(int n,char**v){if(n!=2)return 2;struct A a={2,{{1,7,~0u},{2,7,12345},{4,7,~0u},{16,7,~0u},{32,7,~0u}}};char*keys[]={"system.posix_acl_access","system.posix_acl_default","com.apple.system.Security","user.com.apple.system.Security"};for(int i=0;i<4;i++){errno=0;int r=setxattr(v[1],keys[i],&a,sizeof(a),0);printf("set %s result=%d errno=%d\n",keys[i],r,errno);}errno=0;int fd=open(v[1],O_WRONLY|O_TRUNC);printf("open result=%d errno=%d\n",fd,errno);if(fd>=0){write(fd,"BAD",3);close(fd);}return 0;}
