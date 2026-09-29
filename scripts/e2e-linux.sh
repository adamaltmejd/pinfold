#!/bin/sh
# Run Linux CI on GitHub for HEAD. Push it to a temporary checks/<sha>
# branch over HTTPS, dispatch CI, wait for the verdict, then delete the
# branch. The host uses gh's login, not an SSH agent.
set -eu

repo=adamaltmejd/pinfold
url="https://github.com/$repo.git"
sha=$(git rev-parse HEAD)
branch="checks/$sha"

# The empty helper drops any keychain helper the real HOME configures, and
# -c keeps gh's token out of argv and the user's git config.
git_gh() { git -c credential.helper= -c credential.helper='!gh auth git-credential' "$@"; }

cleanup() { git_gh push -q "$url" ":refs/heads/$branch" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

git_gh push -q "$url" "+$sha:refs/heads/$branch"
gh workflow run CI -R "$repo" --ref "$branch"

run=
while [ -z "$run" ]; do
  sleep 5
  run=$(gh run list -R "$repo" --workflow CI --branch "$branch" \
    --event workflow_dispatch --limit 1 --json databaseId \
    --jq '.[0].databaseId // empty')
done
echo "https://github.com/$repo/actions/runs/$run"

if ! gh run watch -R "$repo" "$run" --exit-status --interval 10 >/dev/null; then
  gh run view -R "$repo" "$run" --log-failed | tail -n 200
  exit 1
fi
