#!/bin/bash
# Stop every prototype process on the lease.
pkill -f '[c]heck-disk.sh'; pkill -f '[m]easure.sh'
pkill -KILL -x firecracker; pkill -f '[f]cjail'; pkill -f '[p]roxy.py'; pkill -x socat
true
