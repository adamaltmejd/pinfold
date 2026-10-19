#define _GNU_SOURCE
#include <sys/types.h>
#include <sys/xattr.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
int main(int n,char**v){if(n!=2)return 2;char b[8192];errno=0;ssize_t r=listxattr(v[1],b,sizeof(b));printf("list r=%zd errno=%d\n",r,errno);for(ssize_t i=0;r>0&&i<r;i+=strlen(b+i)+1)printf("name=%s\n",b+i);char*keys[]={"com.apple.system.Security","user.com.apple.system.Security","system.posix_acl_access","system.posix_acl_default"};for(int i=0;i<4;i++){errno=0;r=getxattr(v[1],keys[i],b,sizeof(b));printf("get %s r=%zd errno=%d\n",keys[i],r,errno);errno=0;r=removexattr(v[1],keys[i]);printf("remove %s r=%zd errno=%d\n",keys[i],r,errno);}errno=0;r=chmod(v[1],0777);printf("chmod r=%zd errno=%d\n",r,errno);errno=0;r=chown(v[1],12345,12345);printf("chown r=%zd errno=%d\n",r,errno);errno=0;int fd=open(v[1],O_WRONLY|O_TRUNC);printf("open r=%d errno=%d\n",fd,errno);if(fd>=0){write(fd,"MUTATED",7);close(fd);}return 0;}
