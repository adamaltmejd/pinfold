use strict; use POSIX qw(setsid); use IO::Handle;
my ($tag)=@ARGV; $SIG{TERM}=$SIG{HUP}=$SIG{INT}='IGNORE';
open my $log,'>',"/tmp/$tag.pids" or die $!; $log->autoflush(1);
pipe(my $r,my $w); $w->autoflush(1);
sub worker {
 my ($role)=@_; close $r;
 open STDIN,'<','/dev/null'; open STDOUT,'>','/dev/null'; open STDERR,'>','/dev/null';
 print $log "$role $$\n"; print $w "ready\n"; close $w;
 while(1){system('curl','-fsS','--max-time','2',"http://receipt/$tag/$role");select undef,undef,undef,0.15;}
}
for my $role(qw(ignore double setsid forker)) {
 my $p=fork(); die $! unless defined $p; next if $p;
 if($role eq 'double'){my $q=fork();exit 0 if $q;}
 setsid() if $role eq 'setsid';
 if($role eq 'forker') {
  my $q=fork(); if(!$q){print $log "churn $$\n";close $w;close $r;while(1){my $v=fork();exit 0 unless defined $v;exit 0 if !$v;waitpid($v,0);select undef,undef,undef,0.01;}}
 }
 worker($role);exit 0;
}
close $w; for(1..4){scalar <$r>;} close $r;
print $log "leader $$\n";close $log; STDOUT->autoflush(1);print "READY $tag\n";
while(1){select undef,undef,undef,10;}
