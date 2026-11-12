# One case per box: `bash cases.sh CASE`. Expects T and P; the driver
# starts the fakes. Each case starts a fresh box with fresh disks, runs the
# agents through `box exec` without --harness, prints what each fake saw
# and the run's egress log, and tears the box down.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/env.sh"
F="$here/files"
CASE=$1
BOX="plp-$CASE"

PI='cd /work && timeout 120 /opt/pinfold/pi/pi -p "say hi" --provider anthropic --model sonnet'
CL='cd /work && timeout 120 /opt/pinfold/claude/claude -p "say hi"'
CLK='cd /work && ANTHROPIC_API_KEY=pinfold-placeholder timeout 120 /opt/pinfold/claude/claude -p "say hi"'
CX='cd /work && timeout 60 /opt/pinfold/codex/codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox "say hi"'

agent() { # agent LABEL SHELL-COMMAND
  echo "== $1"
  echo "\$ box exec $BOX -- sh -c '$2'"
  "$P" box exec "$BOX" -- sh -c "$2" </dev/null 2>&1 | tail -15
  echo "[exit ${PIPESTATUS[0]}]"
  show_fakes
}

common() { # the box's environment and its read-only mounts
  echo "== box env"
  x sh -c 'env | sort | grep -v "^PWD="'
  echo "== mounts"
  x sh -c 'for d in /home/pi/.pi/agent/extensions /opt/plp /etc/claude-code /etc/codex; do [ -e $d ] && { ls -la $d; for f in $d/*; do [ -f $f ] && { echo "--- $f"; cat $f; }; done; touch $d/write-probe 2>&1; }; done; true'
}

homefiles() {
  echo "== files the agents wrote under HOME and /work"
  x sh -c 'find /home/pi /work -type f 2>/dev/null | grep -v "/node_modules/" | sort | head -60'
}

tui() { # tui LABEL KEYS SHELL-COMMAND: an interactive start on a terminal
  echo "== $1 interactive, keys $2"
  KEYS=$2 python3 -I "$here/pty_keys.py" "$T/tty-$1.out" "$P" box exec "$BOX" --tty -- sh -c "$3"
  echo "[exit $?]"
  echo "-- screen text (escapes stripped, last 70 distinct lines):"
  python3 -I "$here/screen.py" "$T/tty-$1.out"
}

up() { # up MOUNT-OR-ENV-ARGS...
  python3 -I "$here/spec.py" "$BOX" "$@" >"$T/$BOX.json"
  echo "== spec"; cat "$T/$BOX.json"; echo
  box_up "$BOX" "$T/$BOX.json"
}

case $CASE in
baseline) # negative controls: no login file at all
  up
  echo "== versions"
  x sh -c '/opt/pinfold/pi/pi --version; /opt/pinfold/claude/claude --version; /opt/pinfold/codex/codex --version'
  common
  agent "pi, no login file" "$PI"
  agent "claude, no login file" "$CL"
  agent "codex, no login file" "$CX"
  egress
  ;;
pi-ext) # Q1: pi's auto-loaded extensions directory, read-only
  up mount="$F/pi-ext:/home/pi/.pi/agent/extensions"
  common
  agent "pi, extension in ~/.pi/agent/extensions" "$PI"
  x sh -c 'ls -lan /home/pi /home/pi/.pi /home/pi/.pi/agent'
  agent "pi, the same with --no-session" "$PI --no-session"
  agent "pi auth --help" 'cd /work && /opt/pinfold/pi/pi auth --help 2>&1 | head -20'
  egress; homefiles
  ;;
pi-ext-raw) # Q1: the extension pointed at the plain route, to see pi's own headers
  up mount="$F/pi-ext-raw:/opt/plp"
  x sh -c 'mkdir -p /home/pi/.pi/agent && printf "{\"extensions\":[\"/opt/plp/pinfold-login.ts\"]}\n" >/home/pi/.pi/agent/settings.json'
  agent "pi, settings.json names the extension to the raw route" "$PI"
  ;;
pi-ext-predir) # Q1: the extensions mount on a home disk where ~/.pi/agent already exists
  up disks=$BOX
  x sh -c 'mkdir -p /home/pi/.pi/agent && ls -lan /home/pi/.pi'
  box_down "$BOX"
  up disks=$BOX mount="$F/pi-ext:/home/pi/.pi/agent/extensions"
  x sh -c 'ls -lan /home/pi/.pi /home/pi/.pi/agent'
  agent "pi, extension mounted below an existing ~/.pi/agent" "$PI"
  egress; homefiles
  ;;
pi-settings) # Q1: settings.json on the home disk names an extension in a read-only mount
  up mount="$F/pi-ext:/opt/plp"
  x sh -c 'mkdir -p /home/pi/.pi/agent && printf "{\"extensions\":[\"/opt/plp/pinfold-login.ts\"]}\n" >/home/pi/.pi/agent/settings.json && cat /home/pi/.pi/agent/settings.json'
  agent "pi, settings.json extensions entry" "$PI"
  egress; homefiles
  x sh -c 'cat /home/pi/.pi/agent/settings.json'
  ;;
pi-models) # Q1: models.json, a symlink on the home disk into a read-only mount
  up mount="$F/pi-models:/opt/plp"
  common
  x sh -c 'mkdir -p /home/pi/.pi/agent && ln -s /opt/plp/models.json /home/pi/.pi/agent/models.json && ls -la /home/pi/.pi/agent'
  agent "pi, models.json symlinked to /opt/plp/models.json" "$PI"
  egress; homefiles
  x sh -c 'ls -la /home/pi/.pi/agent'
  ;;
claude-managed) # Q2: managed settings env block, no ANTHROPIC_API_KEY anywhere
  up mount="$F/claude-managed:/etc/claude-code"
  common
  agent "claude, managed settings only" "$CL"
  agent "claude, ANTHROPIC_API_KEY in claude's own process env" "$CLK"
  egress; homefiles
  ;;
claude-managed-raw) # Q2: the same, to the plain route, to see claude's own headers
  up mount="$F/claude-managed-raw:/etc/claude-code"
  agent "claude, managed settings to the raw route" "$CL"
  agent "claude, raw route, ANTHROPIC_API_KEY in its env" "$CLK"
  ;;
claude-key-boxwide) # Q2: ANTHROPIC_API_KEY box-wide beside the managed file
  up mount="$F/claude-managed:/etc/claude-code" env=ANTHROPIC_API_KEY=pinfold-placeholder
  common
  agent "claude, managed settings, box-wide ANTHROPIC_API_KEY" "$CL"
  ;;
claude-key-boxwide-raw) # Q2: the same to the plain route
  up mount="$F/claude-managed-raw:/etc/claude-code" env=ANTHROPIC_API_KEY=pinfold-placeholder
  agent "claude, raw route, box-wide ANTHROPIC_API_KEY" "$CL"
  ;;
claude-key-blanked) # Q2: box-wide ANTHROPIC_API_KEY, managed file sets it empty
  up mount="$F/claude-managed-key:/etc/claude-code" env=ANTHROPIC_API_KEY=pinfold-placeholder
  common
  agent "claude, managed settings blank ANTHROPIC_API_KEY, box-wide one present" "$CL"
  ;;
codex) # Q3: /etc/codex/config.toml alone
  up mount="$F/codex:/etc/codex"
  common
  agent "codex, /etc/codex/config.toml" "$CX"
  egress; homefiles
  ;;
all) # Q4, Q5: all three files in one box, one box-wide env. pi's
  # extensions mount needs ~/.pi/agent to exist first (case pi-ext).
  up disks=$BOX
  x sh -c 'mkdir -p /home/pi/.pi/agent'
  box_down "$BOX"
  up disks=$BOX mount="$F/pi-ext:/home/pi/.pi/agent/extensions" \
    mount="$F/claude-managed:/etc/claude-code" mount="$F/codex:/etc/codex"
  common
  agent "pi, all three present" "$PI"
  agent "claude, all three present" "$CL"
  agent "codex, all three present" "$CX"
  agent "pi given claude's two variables, as a child of claude would be" \
    "cd /work && ANTHROPIC_BASE_URL=http://claude CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder timeout 120 /opt/pinfold/pi/pi -p 'say hi' --provider anthropic --model sonnet"
  agent "codex given claude's two variables" \
    "cd /work && ANTHROPIC_BASE_URL=http://claude CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder timeout 60 /opt/pinfold/codex/codex exec --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox 'say hi'"
  agent "pi auth check" 'cd /work && timeout 60 /opt/pinfold/pi/pi auth check --provider anthropic 2>&1 | head -20'
  agent "codex login status" 'cd /work && timeout 60 /opt/pinfold/codex/codex login status 2>&1 | head'
  egress; homefiles
  tui pi '[[8,"/login\r"],[6,"\u001b"],[2,"\u0003"],[1,"\u0003"]]' 'cd /work && /opt/pinfold/pi/pi'
  tui claude '[[10,"\r"],[5,"\r"],[5,"\r"],[5,"/login\r"],[6,"\u001b"],[2,"\u0003"],[1,"\u0003"]]' 'cd /work && /opt/pinfold/claude/claude'
  tui codex '[[10,"/login\r"],[5,"\u001b"],[2,"\u0003"],[1,"\u0003"]]' 'cd /work && /opt/pinfold/codex/codex --dangerously-bypass-approvals-and-sandbox'
  tui codex-no-daemon '[[10,"/login\r"],[5,"\u001b"],[2,"\u0003"],[1,"\u0003"]]' 'cd /work && /opt/pinfold/codex/codex --no-daemon --dangerously-bypass-approvals-and-sandbox'
  show_fakes
  homefiles
  common
  ;;
claude-tui-env) # Q5 control: claude's first interactive start with today's
  # per-exec variables in its process env and no managed file
  up
  tui claude-process-env '[[10,"\r"],[5,"\r"],[5,"\r"],[5,"/login\r"],[6,"\u001b"],[2,"\u0003"],[1,"\u0003"]]' 'cd /work && ANTHROPIC_BASE_URL=http://claude CLAUDE_CODE_OAUTH_TOKEN=pinfold-placeholder /opt/pinfold/claude/claude'
  show_fakes; egress
  ;;
*) echo "unknown case $CASE"; exit 2 ;;
esac
box_down "$BOX"
