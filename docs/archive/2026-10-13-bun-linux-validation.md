# Bun Linux validation, 2026-10-13

After explicit approval for the temporary GitHub upload, candidate
`dcb8b38c8d543c7f33b535f540340efdc0cffea5` passed
[CI run 38387117755](https://github.com/adamaltmejd/pinfold/actions/runs/38387117755).
The candidate contains the all-pin updater changes and Bun 1.4.3 migration.
Its tracked task files match the original working checkout.

| Platform | Rust end-to-end step | Bun durable step | Result |
|---|---:|---:|---|
| Linux arm64 | 226 seconds | 12 seconds | Passed |
| Linux x64 | 278 seconds | 20 seconds | Passed |

Both jobs also passed formatting and Clippy. Each Bun step installed the
frozen lockfile, ran the Bun-only typecheck, then executed guarantee 35
against real rootless Podman boxes. Together with the earlier Mac results
in `2026-10-13-test-audit.md`, all three runtime gates passed. Both Linux
suite steps stayed below the five-minute CI budget.

The helper deleted `checks/dcb8b38c8d543c7f33b535f540340efdc0cffea5`;
a GitHub ref lookup returned 404 afterward. The temporary validation
worktree was retired. The original checkout retains the task changes
for review. No main-branch push, pull request or release was made.
