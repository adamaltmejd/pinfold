use strict; use warnings; use POSIX qw(setsid pause);
$|=1; my $mode=shift // 'plain';
if ($mode eq 'peer') { print "READY:$$\n"; while (<STDIN>) { print "B-OK:$_"; } exit 0; }
if ($mode eq 'ignore') { $SIG{TERM}='IGNORE'; }
if ($mode eq 'escape' || $mode eq 'background') {
 pipe(my $r,my $w) or die $!; my $pid=fork(); die $! unless defined $pid;
 if (!$pid) { close $r; setsid() if $mode eq 'escape'; $SIG{HUP}='IGNORE'; print $w "ready\n"; close $w;
   if ($mode eq 'background') { close STDIN; }
   while (1) { if ($mode eq 'background' && -e '/tmp/write-background') { print "background-output\n"; unlink '/tmp/write-background'; open my $f,'>','/tmp/background-wrote'; print $f "OK\n"; close $f; } select undef,undef,undef,0.01; }
 }
 close $w; scalar <$r>; close $r; print "READY:$$:$pid\n"; exit 0 if $mode eq 'background';
} else { print "READY:$$\n"; }
if ($mode eq 'eof') { scalar <STDIN>; print "STDIN-EOF\n"; }
while (1) { pause(); }
