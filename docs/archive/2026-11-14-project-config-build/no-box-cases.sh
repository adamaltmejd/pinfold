#!/bin/bash
# No-box cases for #294. Usage: no-box-cases.sh PATH/TO/pinfold
#
# Every case is refused before a box, or reads only. The XDG config and
# state roots are under a fresh temporary directory; each state root gets
# a fresh maintenance stamp first, so the daily pass (which prunes dead
# boxes and old builds host-wide) does not run. Cases that start a box
# are in run-box-cases.sh.
set -u
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
W=$(mktemp -d "${TMPDIR:-/tmp}/pf.XXXX")
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
unset VISUAL EDITOR PINFOLD_PROFILE PINFOLD_ALLOW PINFOLD_ROUTES PINFOLD_CPUS PINFOLD_MEMORY
for v in $(compgen -e | grep '^PINFOLD_ENV_'); do unset "$v"; done
stamp() { mkdir -p "$1/pinfold" && touch "$1/pinfold/maintenance"; }
stamp "$W/state"
pinfold() { "$BIN" "$@"; }
run() { echo "\$ $*"; "$@" </dev/null 2>&1; echo "[exit $?]"; echo; }
git init -q "$W/proj" && git -C "$W/proj" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
mkdir "$W/plain"
cd "$W/proj"

echo "## 1. No configuration"
run pinfold config
run pinfold pi
run pinfold build
run pinfold push
run pinfold fetch
echo "config root after refusals:"; find "$W/config" 2>&1; echo
echo "project state dirs: $(ls "$W/state/pinfold/projects" 2>/dev/null | wc -l | tr -d ' ')"; echo
echo "doctor without configuration (grep of its config line and exit):"
pinfold doctor </dev/null 2>&1 | grep -E '^(pinfold doctor|config):'; echo "[exit ${PIPESTATUS[0]}]"; echo

echo "## 2. A .pinfold.toml in the project is not read"
printf 'allow = ["example.com"]\n' > .pinfold.toml
run pinfold config
rm .pinfold.toml

echo "## 3. pinfold allow is gone"
run pinfold allow

echo "## 4. config edit"
run pinfold config edit --help
EDITOR=true run pinfold config edit
F=$(ls "$W"/config/pinfold/projects/*/pinfold.toml); D=$(dirname "$F"); PF=$(basename "$D")
echo "file bytes:"; cat "$F"; echo "entries beside it: $(ls -A "$D")"; echo
run pinfold config
printf 'allow = ["only.example"]\n' >> "$F"; cp "$F" "$W/good"
EDITOR=true run pinfold config edit
cmp "$F" "$W/good" && echo "existing file unchanged"; echo
EDITOR=false run pinfold config edit
VISUAL="head -n1" EDITOR=false run pinfold config edit

echo "## 5. Mount validation"
for body in 'mounts = [{ host = ".", kind = "share" }]' 'mounts = [{ host = "/tmp", kind = "disk" }]' \
  'mounts = "x"' 'mounts = [{ host = 1, kind = "disk" }]' 'mounts = [{ host = "." }]' \
  'mounts = [{ host = ".", kind = "disk", guest = "/x" }]' \
  'mounts = [{ host = ".", kind = "disk" }, { host = ".", kind = "disk" }]' 'allow = []'; do
  printf '%s\n' "$body" > "$F"; echo "file: $body"; run pinfold config
done
printf '[[mounts]]\nhost = "."\nkind = "disk"\n' > "$F"; echo "file: the [[mounts]] table form"; run pinfold config
printf 'mounts = [{ host = ".", kind = "disk" }]\ncontainerfile = "Containerfile"\n' > "$F"; echo "file: old containerfile key"; run pinfold config
cp "$W/good" "$F"

echo "## 6. A profile's mounts"
mkdir -p "$W/config/pinfold/profiles/withmounts"
printf 'mounts = [{ host = ".", kind = "disk" }]\n' > "$W/config/pinfold/profiles/withmounts/pinfold.toml"
PINFOLD_PROFILE=withmounts run pinfold config
rm -r "$W/config/pinfold/profiles/withmounts"

echo "## 7. Project recipe beside the file"
printf 'FROM localhost/pinfold/profile-default:latest\n' > "$D/Containerfile"
run pinfold config
printf 'image = "localhost/x:latest"\n' >> "$F"
run pinfold config
cp "$W/good" "$F"; rm "$D/Containerfile"
printf 'image = "localhost/x:latest"\n' >> "$F"
run pinfold build
cp "$W/good" "$F"

echo "## 8. Dangling configuration symlink"
mv "$F" "$W/moved.toml"; ln -s "$W/proj/target.toml" "$F"
run pinfold config
EDITOR=true run pinfold config edit
echo "proj/target.toml exists: $([ -e "$W/proj/target.toml" ] && echo yes || echo no)"; echo
rm "$F"; ln -s "$W/outside.toml" "$F"
echo "dangling link to a path outside the project:"
EDITOR=true run pinfold config edit
echo "outside.toml exists: $([ -e "$W/outside.toml" ] && echo yes || echo no)"; echo
rm "$F"; mv "$W/moved.toml" "$F"

echo "## 9. Global pinfold.toml is not read"
printf 'allow = ["global.example"]\n' > "$W/config/pinfold/pinfold.toml"
sed -i '' '/only.example/d' "$F"
run pinfold config
rm "$W/config/pinfold/pinfold.toml"

echo "## 10. A moved copy is a new project"
cp -R "$W/proj" "$W/proj-moved"; cd "$W/proj-moved"
run pinfold config
cd "$W/proj"

echo "## 11. host-path-in-project"
mkdir -p "$W/proj/link-target"; ln -s "$W/proj/link-target" "$W/cachelink"
echo "cache root inside the project through a symlinked ancestor whose suffix does not exist:"
XDG_CACHE_HOME=$W/cachelink/missing/cache run pinfold pi
XDG_CACHE_HOME=$W/cachelink/missing/cache run pinfold build
XDG_CACHE_HOME=$W/cachelink/missing/cache run pinfold push
echo "under link-target: $(find "$W/proj/link-target" -mindepth 1 | wc -l | tr -d ' ') entries"; echo
echo "config root inside the project, for config edit:"
XDG_CONFIG_HOME=$W/proj/cfg EDITOR=true run pinfold config edit
echo "proj/cfg exists: $([ -e "$W/proj/cfg" ] && echo yes || echo no)"; echo
mkdir -p "$W/config3/pinfold/projects/$PF"
cp "$F" "$W/proj/policy.toml"; ln -s "$W/proj/policy.toml" "$W/config3/pinfold/projects/$PF/pinfold.toml"
echo "the project's file symlinked into the project:"
XDG_CONFIG_HOME=$W/config3 run pinfold pi
XDG_CONFIG_HOME=$W/config3 run pinfold build
XDG_CONFIG_HOME=$W/config3 run pinfold push
rm "$W/config3/pinfold/projects/$PF/pinfold.toml"; cp "$F" "$W/config3/pinfold/projects/$PF/pinfold.toml"
printf 'FROM localhost/pinfold/profile-default:latest\n' > "$W/proj/Recipe"; ln -s "$W/proj/Recipe" "$W/config3/pinfold/projects/$PF/Containerfile"
echo "the project's Containerfile symlinked into the project:"
XDG_CONFIG_HOME=$W/config3 run pinfold pi
XDG_CONFIG_HOME=$W/config3 run pinfold build
rm "$W/config3/pinfold/projects/$PF/Containerfile"
mkdir -p "$W/config3/pinfold/profiles/p"
printf 'FROM scratch\n' > "$W/config3/pinfold/profiles/p/Containerfile"
printf 'allow = ["from.project"]\n' > "$W/proj/profile.toml"
ln -s "$W/proj/profile.toml" "$W/config3/pinfold/profiles/p/pinfold.toml"
echo "the profile's pinfold.toml symlinked into the project:"
XDG_CONFIG_HOME=$W/config3 PINFOLD_PROFILE=p run pinfold pi
XDG_CONFIG_HOME=$W/config3 PINFOLD_PROFILE=p run pinfold build

echo "## 12. A folder name with a newline"
N="$W/new
line"; mkdir "$N"; git init -q "$N"; cd "$N"
EDITOR=true run pinfold config edit
echo "file bytes:"; cat "$(ls -d "$W"/config/pinfold/projects/new*line*/ | head -1)pinfold.toml" 2>&1 || cat "$W"/config/pinfold/projects/new-line-*/pinfold.toml; echo

echo "## 13. macOS outside Git"
cd "$W/plain"
run pinfold config
EDITOR=true run pinfold config edit
run pinfold config
echo "workdir: $W"
