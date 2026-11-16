#define _GNU_SOURCE
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <sys/timerfd.h>
#include <linux/capability.h>
#include <sched.h>
#include <grp.h>
#include <dirent.h>

#include <signal.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#ifndef WORK_UID
#define WORK_UID 1000
#define WORK_GID 1000
#endif
static const char *cg="/sys/fs/cgroup";
static int put(const char *p,const char *v){int f=open(p,O_WRONLY);if(f<0)return -1;int n=write(f,v,strlen(v)),e=errno;close(f);errno=e;return n<0?-1:0;}
static void caps(unsigned m){struct __user_cap_header_struct h={_LINUX_CAPABILITY_VERSION_3,0};struct __user_cap_data_struct d[2]={{m,m,0},{0,0,0}};if(syscall(SYS_capset,&h,d))perror("capset");}
static void status(const char *who){char b[256];FILE*f=fopen("/proc/self/status","r");printf("%s pid=%d ",who,getpid());while(fgets(b,sizeof b,f))if(!strncmp(b,"Uid:",4)||!strncmp(b,"Gid:",4)||!strncmp(b,"Cap",3)||!strncmp(b,"NoNewPrivs:",11)){b[strcspn(b,"\n")]=0;printf("%s;",b);}fclose(f);puts("");}
static void delay(unsigned ms){int f=timerfd_create(CLOCK_MONOTONIC,0);struct itimerspec t={.it_value={ms/1000,(ms%1000)*1000000}};timerfd_settime(f,0,&t,0);unsigned long long v;read(f,&v,8);close(f);}
static long long ns(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1000000000LL+t.tv_nsec;}
static void work(int fd){
 signal(SIGTERM,SIG_IGN);signal(SIGHUP,SIG_IGN);signal(SIGINT,SIG_IGN);
 for(int role=0;role<4;role++){pid_t p=fork();if(p)continue;if(role==1){if(fork())_exit(0);}if(role==2)setsid();write(fd,"r",1);close(fd);for(;;){if(role==3){pid_t q=fork();if(!q)_exit(0);waitpid(q,0,0);}else pause();}}
 write(fd,"r",1);close(fd);for(;;)pause();
}
static int ns_count(ino_t target){DIR*d=opendir("/proc");struct dirent*e;int n=0;char p[128];struct stat st;while((e=readdir(d))){if(e->d_name[0]<'0'||e->d_name[0]>'9')continue;snprintf(p,sizeof p,"/proc/%s/ns/pid",e->d_name);if(!stat(p,&st)&&st.st_ino==target)n++;}closedir(d);return n;}
static void tree(pid_t root,pid_t *list,int *n){DIR*d=opendir("/proc");struct dirent*e;char p[128],line[256];while((e=readdir(d))&&*n<256){if(e->d_name[0]<'0'||e->d_name[0]>'9')continue;snprintf(p,sizeof p,"/proc/%s/status",e->d_name);FILE*f=fopen(p,"r");if(!f)continue;int parent=-1;while(fgets(line,sizeof line,f))sscanf(line,"PPid: %d",&parent);fclose(f);if(parent==root){int child=atoi(e->d_name);list[(*n)++]=child;tree(child,list,n);}}closedir(d);}
static void alternatives(void){
 int f[2];pipe(f);pid_t helper=fork();if(!helper){close(f[0]);if(unshare(CLONE_NEWPID)){int v=-errno;write(f[1],&v,sizeof v);_exit(0);}int gate[2];pipe(gate);pid_t init=fork();if(!init){close(gate[1]);char go;read(gate[0],&go,1);close(gate[0]);work(f[1]);_exit(0);}close(gate[0]);write(f[1],&init,sizeof init);write(gate[1],"g",1);close(gate[1]);close(f[1]);waitpid(init,0,0);_exit(0);}close(f[1]);int init=0;read(f[0],&init,sizeof init);
 if(init>0){char b[8],p[80];int n=0;while(n<5){int k=read(f[0],b,5-n);if(k<=0)break;n+=k;}snprintf(p,sizeof p,"/proc/%d/ns/pid",init);struct stat st;stat(p,&st);int before=ns_count(st.st_ino);kill(init,SIGKILL);waitpid(helper,0,0);printf("PIDNS init=%d ready=%d before=%d after=%d\n",init,n,before,ns_count(st.st_ino));}else{waitpid(helper,0,0);printf("PIDNS unshare errno=%d\n",-init);}close(f[0]);
 pid_t supervisor=fork();if(!supervisor){prctl(PR_SET_CHILD_SUBREAPER,1,0,0,0);int gate[2],report[2];pipe(gate);pipe(report);pid_t a=fork();if(!a){close(gate[1]);close(report[0]);char c;read(gate[0],&c,1);pid_t b=fork();if(!b){if(fork())_exit(0);setsid();pid_t me=getpid();write(report[1],&me,sizeof me);for(;;)pause();}for(;;)pause();}close(gate[0]);close(report[1]);pid_t list[256];int count=0;tree(getpid(),list,&count);printf("TREE snapshot-count=%d first=%d ",count,count?list[0]:-1);write(gate[1],"g",1);close(gate[1]);pid_t escaped;read(report[0],&escaped,sizeof escaped);close(report[0]);for(int i=0;i<count;i++)kill(list[i],SIGKILL);waitpid(a,0,0);char p[80],line[256];int parent=0;for(int i=0;i<10000&&parent!=getpid();i++){snprintf(p,sizeof p,"/proc/%d/status",escaped);FILE*f=fopen(p,"r");while(fgets(line,sizeof line,f))sscanf(line,"PPid: %d",&parent);fclose(f);sched_yield();}printf("post-snapshot-doublefork=%d alive=%d subreaper=%d observed-parent=%d\n",escaped,kill(escaped,0)==0,getpid(),parent);kill(escaped,SIGKILL);while(waitpid(-1,0,0)>0){} _exit(0);}waitpid(supervisor,0,0);
}
int main(int argc,char **argv){
 if(argc>1){status("after-exec");work(atoi(argv[1]));return 0;}
 setbuf(stdout,0);if(getpid()==1){mkdir("/proc",0755);mount("proc","/proc","proc",0,0);mkdir("/sys",0755);mount("sysfs","/sys","sysfs",0,0);mkdir("/sys/fs/cgroup",0755);mount("cgroup2",cg,"cgroup2",0,0);mkdir("/tmp",01777);mount("tmpfs","/tmp","tmpfs",0,0);put("/proc/sys/user/max_user_namespaces","0");}
 status("initial");
#ifdef ALTERNATIVES
 alternatives();
#endif
mkdir("/sys/fs/cgroup/probe-controller",0755);printf("mkdir-before-drop=%d errno=%d\n",mkdir("/sys/fs/cgroup/probe-controller/probe-scope",0755),errno);
 for(int c=0;c<=40;c++)prctl(PR_CAPBSET_DROP,c,0,0,0);caps((1U<<CAP_SETUID)|(1U<<CAP_SETGID)
#ifdef KEEP_KILL
 |(1U<<CAP_KILL)
#endif
 );prctl(PR_SET_NO_NEW_PRIVS,1,0,0,0);status("controller");
 char b[512];FILE*f=fopen("/sys/fs/cgroup/cgroup.controllers","r");if(f){fgets(b,sizeof b,f);printf("controllers=%s",b);fclose(f);}f=fopen("/sys/fs/cgroup/cgroup.max.descendants","r");if(f){fgets(b,sizeof b,f);printf("max.descendants=%s",b);fclose(f);}
 int m=mkdir("/sys/fs/cgroup/probe-controller/probe-after-drop",0755);printf("mkdir-after-drop=%d errno=%d\n",m,errno);rmdir("/sys/fs/cgroup/probe-controller/probe-after-drop");
 mkdir("/sys/fs/cgroup/probe-controller/peer",0755);int peer_ready[2];pipe(peer_ready);pid_t peer=fork();if(!peer){close(peer_ready[0]);put("/sys/fs/cgroup/probe-controller/peer/cgroup.procs","0");setgroups(0,0);setgid(WORK_GID);setuid(WORK_UID);caps(0);write(peer_ready[1],"r",1);close(peer_ready[1]);for(;;)pause();}close(peer_ready[1]);read(peer_ready[0],b,1);close(peer_ready[0]);
 int ready[2];pipe(ready);pid_t child=fork();
 if(!child){close(ready[0]);if(put("/sys/fs/cgroup/probe-controller/probe-scope/cgroup.procs","0")){perror("move-self");_exit(2);}setgroups(0,0);if(setgid(WORK_GID)||setuid(WORK_UID)){perror("identity-drop");_exit(3);}caps(0);status("workload");errno=0;int u=unshare(CLONE_NEWPID);printf("unprivileged-pidns=%d errno=%d\n",u,errno);errno=0;int e=put("/sys/fs/cgroup/cgroup.procs","0");printf("escape-root=%d errno=%d\n",e,errno);errno=0;e=put("/sys/fs/cgroup/probe-controller/probe-scope/cgroup.procs","0");printf("write-own=%d errno=%d\n",e,errno);errno=0;e=mkdir("/sys/fs/cgroup/evil",0755);printf("mkdir-workload=%d errno=%d\n",e,errno);char fd[24];snprintf(fd,sizeof fd,"%d",ready[1]);execl("/proc/self/exe","controller",fd,(char*)0);perror("exec");_exit(4);}
 close(ready[1]);int n=0;while(n<5){int v=read(ready[0],b,5-n);if(v<=0)break;n+=v;}close(ready[0]);printf("ready-count=%d\n",n);
 f=fopen("/sys/fs/cgroup/probe-controller/probe-scope/cgroup.procs","r");int p,count=0,denied=0;while(fscanf(f,"%d",&p)==1){count++;if(kill(p,SIGTERM)&&errno==EPERM)denied++;}fclose(f);printf("TERM members=%d denied-without-CAP_KILL=%d\n",count,denied);delay(2000);
 long long start=ns();int k=put("/sys/fs/cgroup/probe-controller/probe-scope/cgroup.kill","1");printf("cgroup.kill=%d errno=%d us=%lld\n",k,errno,(ns()-start)/1000);
 for(int i=0;i<10000;i++){while(waitpid(-1,0,WNOHANG)>0){}f=fopen("/sys/fs/cgroup/probe-controller/probe-scope/cgroup.events","r");int populated=1;while(fgets(b,sizeof b,f))sscanf(b,"populated %d",&populated);fclose(f);if(!populated){printf("cgroup.events populated 0 empty-us=%lld\n",(ns()-start)/1000);break;}if(i==9999)puts("EMPTY-NOT-OBSERVED");sched_yield();}
 printf("peer-exists=%d\n",kill(peer,0)==0||errno==EPERM);put("/sys/fs/cgroup/probe-controller/peer/cgroup.kill","1");waitpid(peer,0,0);rmdir("/sys/fs/cgroup/probe-controller/peer");
 rmdir("/sys/fs/cgroup/probe-controller/probe-scope");start=ns();int ok=0;for(int i=0;i<1000;i++){if(!mkdir("/sys/fs/cgroup/probe-controller/probe-cost",0755)&&!rmdir("/sys/fs/cgroup/probe-controller/probe-cost"))ok++;}printf("empty-create-remove n=%d mean-us=%lld\n",ok,(ns()-start)/1000000);
 int concurrent=0;for(int i=0;i<256;i++){snprintf(b,sizeof b,"/sys/fs/cgroup/probe-controller/probe-%d",i);if(mkdir(b,0755))break;concurrent++;}printf("concurrent-empty=%d\n",concurrent);for(int i=0;i<concurrent;i++){snprintf(b,sizeof b,"/sys/fs/cgroup/probe-controller/probe-%d",i);rmdir(b);}puts("PROBE-DONE");if(getpid()==1)for(;;)pause();return 0;
}
