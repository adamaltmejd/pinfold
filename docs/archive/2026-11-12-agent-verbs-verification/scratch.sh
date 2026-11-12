# Scratch: outside Git, pi removes empty disks; claude retains them.
# Run with PROJ naming a directory that is not in a Git repository.
. "$(dirname "$0")/env.sh"
for a in pi claude; do
  echo "== pinfold $a --version outside Git"
  "$P" "$a" --version </dev/null 2>&1 | grep -v image-outdated
done
echo "== scratch disks left"
"$P" disk list | grep -E '^NAME|scratch|work' || true
