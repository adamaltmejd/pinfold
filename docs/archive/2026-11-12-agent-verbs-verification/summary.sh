# Case 2b: only pi prints the exit summary. Leaves an untracked file on the
# project disk through attach, then compares pi's and claude's stderr.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/env.sh"
python3 -I "$here/pty_run.py" "$T/out.summary" "$P" pi &
until "$P" attach true </dev/null >/dev/null 2>&1; do sleep 1; done
"$P" attach sh -c 'echo work >untracked.txt' </dev/null
kill -TERM "$(pgrep -f "^$P pi\$")"
wait
for a in pi claude codex; do
  echo "== pinfold $a --version, stderr"
  "$P" "$a" --version </dev/null 2>&1 >/dev/null | grep -v image-outdated
done
