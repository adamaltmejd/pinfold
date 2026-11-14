#!/bin/bash
# No-box cases for #294. Runs the debug binary with XDG config and state
# roots under a fresh temporary directory.
set -u
BIN=$1
W=$(mktemp -d "$TMPDIR/pf.XXXX")
export XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state PINFOLD_NO_UPDATE_CHECK=1
unset VISUAL EDITOR PINFOLD_PROFILE PINFOLD_ALLOW PINFOLD_ROUTES
run() { echo "\$ $*"; "$@" 2>&1; echo "[exit $?]"; echo; }
git init -q "$W/proj" && git -C "$W/proj" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
mkdir "$W/plain"
cd "$W/proj"
echo "## 1. No configuration"
run $BIN config
run $BIN pi
run $BIN build
run $BIN push
run $BIN fetch
run $BIN doctor
echo "state/config trees after refusals:"; find "$W/config" "$W/state" 2>&1; echo
echo "## 2. A .pinfold.toml in the project is not read"
printf 'allow = ["example.com"]\n' > .pinfold.toml
run $BIN config
rm .pinfold.toml
echo "## 3. pinfold allow is gone"
run $BIN allow
echo "## 4. config edit creates the file, editor exit passes through"
run $BIN config edit --help
EDITOR=true run $BIN config edit
F=$(ls "$W"/config/pinfold/projects/*/pinfold.toml); echo "file: $F"; cat "$F"; echo
run $BIN config
printf 'allow = ["only.example"]\n' >> "$F"; cp "$F" "$W/before"
EDITOR=true run $BIN config edit
cmp "$F" "$W/before" && echo "existing file unchanged"; echo
EDITOR=false run $BIN config edit
VISUAL="head -n1" EDITOR=false run $BIN config edit
echo "## 5. Mount validation"
cp "$W/before" "$W/good"
printf 'mounts = [{ host = ".", kind = "share" }]\n' > "$F"; echo "kind share:"; run $BIN config
printf 'mounts = [{ host = "/tmp", kind = "disk" }]\n' > "$F"; echo "host /tmp:"; run $BIN config
printf '[[mounts]]\nhost = "."\nkind = "disk"\n' > "$F"; echo "table form of the project mount:"; run $BIN config
printf 'allow = []\n' > "$F"; echo "no mounts:"; run $BIN config
printf 'mounts = [{ host = ".", kind = "disk" }, { host = ".", kind = "disk" }]\n' > "$F"; echo "two mounts:"; run $BIN config
printf 'mounts = [{ host = ".", kind = "disk", guest = "/x" }]\n' > "$F"; echo "unknown mount key:"; run $BIN config
printf 'mounts = [{ host = ".", kind = "disk" }]\ncontainerfile = "Containerfile"\n' > "$F"; echo "old containerfile key:"; run $BIN config
cp "$W/good" "$F"
echo "## 6. A profile's mounts"
mkdir -p "$W/config/pinfold/profiles/withmounts"
printf 'mounts = [{ host = ".", kind = "disk" }]\n' > "$W/config/pinfold/profiles/withmounts/pinfold.toml"
PINFOLD_PROFILE=withmounts run $BIN config
echo "## 7. Project recipe beside the file"
D=$(dirname "$F")
printf 'FROM localhost/pinfold/profile-default:latest\n' > "$D/Containerfile"
run $BIN config
printf 'image = "localhost/x:latest"\n' >> "$F"
run $BIN config
cp "$W/good" "$F"; rm "$D/Containerfile"
echo "## 8. Dangling configuration symlink"
mv "$F" "$W/moved.toml"; ln -s "$W/nowhere.toml" "$F"
run $BIN config
rm "$F"; mv "$W/moved.toml" "$F"
echo "## 9. Global pinfold.toml is not read"
printf 'allow = ["global.example"]\n' > "$W/config/pinfold/pinfold.toml"
sed -i '' '/only.example/d' "$F"
run $BIN config
rm "$W/config/pinfold/pinfold.toml"
echo "## 10. A moved copy is a new project"
cp -R "$W/proj" "$W/proj-moved"; cd "$W/proj-moved"
run $BIN config
cd "$W/proj"
echo "## 11. host-path-in-project"
mkdir -p "$W/proj/link-target"; ln -s "$W/proj/link-target" "$W/statelink"
PF=$(basename "$D")
# The state root sits inside the project through a symlinked ancestor whose suffix does not exist.
mkdir -p "$W/config2/pinfold/projects"; cp -R "$D" "$W/config2/pinfold/projects/$PF"
XDG_STATE_HOME=$W/statelink/missing/state XDG_CONFIG_HOME=$W/config2 run $BIN pi
XDG_STATE_HOME=$W/statelink/missing/state XDG_CONFIG_HOME=$W/config2 run $BIN build
XDG_STATE_HOME=$W/statelink/missing/state XDG_CONFIG_HOME=$W/config2 run $BIN push
echo "created under the symlinked state root (the daily maintenance pass runs before the verb):"; find "$W/proj/link-target" ; echo
echo "## 11b. The project's file or Containerfile symlinked into the project"
mkdir -p "$W/config3/pinfold/projects/$PF"
cp "$F" "$W/proj/policy.toml"; ln -s "$W/proj/policy.toml" "$W/config3/pinfold/projects/$PF/pinfold.toml"
XDG_CONFIG_HOME=$W/config3 run $BIN pi
XDG_CONFIG_HOME=$W/config3 run $BIN build
XDG_CONFIG_HOME=$W/config3 run $BIN push
rm "$W/config3/pinfold/projects/$PF/pinfold.toml"; cp "$F" "$W/config3/pinfold/projects/$PF/pinfold.toml"
printf 'FROM localhost/pinfold/profile-default:latest\n' > "$W/proj/Recipe"; ln -s "$W/proj/Recipe" "$W/config3/pinfold/projects/$PF/Containerfile"
XDG_CONFIG_HOME=$W/config3 run $BIN pi
XDG_CONFIG_HOME=$W/config3 run $BIN build
echo "control: the same project with external roots passes the check:"
run $BIN pi
run $BIN build
echo "## 12. macOS outside Git"
cd "$W/plain"
run $BIN config
EDITOR=true run $BIN config edit
run $BIN config
run $BIN pi
echo "workdir: $W"
