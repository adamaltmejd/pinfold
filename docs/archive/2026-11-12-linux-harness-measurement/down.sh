#!/bin/bash
# usage: down.sh NAME; ends the box, its `box up` and the stdin sleeper.
"$P" box down "$1"
while pgrep -f "pinfold box up" > /dev/null; do sleep 0.1; done
pkill -f "sleep 3617"
true
