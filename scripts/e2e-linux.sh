#!/bin/sh
# The Linux end-to-end suite on GitHub's runners, for the ref being gated.
# Pushes HEAD to a queue/<sha> branch, dispatches CI at it, waits for the
# verdict and deletes the branch. Nothing of the candidate runs here.
set -eu

repo=adamaltmejd/pinfold
url="git@github.com:$repo.git"
sha=$(git rev-parse HEAD)
branch="queue/$sha"

cleanup() { git push -q "$url" ":refs/heads/$branch" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

git push -q "$url" "+$sha:refs/heads/$branch"
gh workflow run CI -R "$repo" --ref "$branch"

run=
while [ -z "$run" ]; do
  sleep 5
  run=$(gh run list -R "$repo" --workflow CI --branch "$branch" \
    --event workflow_dispatch --limit 1 --json databaseId \
    --jq '.[0].databaseId // empty')
done
echo "https://github.com/$repo/actions/runs/$run"

if ! gh run watch -R "$repo" "$run" --exit-status --interval 30 >/dev/null; then
  gh run view -R "$repo" "$run" --log-failed | tail -n 200
  exit 1
fi
