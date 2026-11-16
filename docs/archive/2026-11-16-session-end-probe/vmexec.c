#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static void dump(const char *tag,const char *path){
 FILE *f=fopen(path,"r");char *s=NULL;size_t n=0;fprintf(stderr,"VMEXEC_PROBE %s ",tag);
 if(!f){perror(path);return;}
 while(getline(&s,&n,f)>0)if(!strncmp(s,"Name:",5)||!strncmp(s,"PPid:",5)||!strncmp(s,"Uid:",4)||!strncmp(s,"Gid:",4)||!strncmp(s,"Cap",3)||!strncmp(s,"NoNewPrivs:",11)||!strncmp(s,"NSpid:",6)){s[strcspn(s,"\n")]=0;fprintf(stderr,"%s; ",s);}
 free(s);fclose(f);fputc('\n',stderr);fflush(stderr);
}
int main(int argc,char **argv){
 (void)argc;char p[80];dump("self","/proc/self/status");snprintf(p,sizeof p,"/proc/%ld/status",(long)getppid());dump("parent",p);dump("pid1","/proc/1/status");
 execv("/sbin/vmexec.real",argv);perror("exec vmexec.real");return 127;
}
