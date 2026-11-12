"""Shared helpers for the 2026-11-12 share concurrency probe.

Env: PFS_CT (container name), PFS_ROOT (host dir shared at /work), PFS_GROOT.
Host and guest git both run with no global config, so the host's signing
and hooks stay out of the trials.
"""
import json, os, subprocess, sys, time

CT = os.environ["PFS_CT"]
ROOT = os.environ["PFS_ROOT"]
# Where the guest sees ROOT; case 7 uses a second box that mounts it at its host path.
GROOT = os.environ.get("PFS_GROOT", "/work")
HENV = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1",
            GIT_AUTHOR_NAME="host", GIT_AUTHOR_EMAIL="h@x",
            GIT_COMMITTER_NAME="host", GIT_COMMITTER_EMAIL="h@x")
GPRE = ("export GIT_CONFIG_GLOBAL=/dev/null GIT_AUTHOR_NAME=guest GIT_AUTHOR_EMAIL=g@x "
        "GIT_COMMITTER_NAME=guest GIT_COMMITTER_EMAIL=g@x "
        "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0='*'; ")
_seq = [0]


def now():
    return time.monotonic()


def h(repo, cmd, env=None):
    """Run a bash command on the host in ROOT/repo. Returns (rc, out)."""
    r = subprocess.run(["bash", "-c", cmd], cwd=os.path.join(ROOT, repo),
                       env=env or HENV, capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def g(repo, cmd):
    """Run a sh command in the guest in /work/repo. Returns (rc, out)."""
    r = subprocess.run(["container", "exec", CT, "sh", "-c",
                        GPRE + f"cd {GROOT}/{repo} && {{ {cmd} ; }}"],
                       capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def new_repo(tag, init_args="", commits=2, cfg=()):
    """Fresh host repository with `commits` commits; files a, b. Returns name."""
    _seq[0] += 1
    name = f"{tag}{os.getpid() % 10000}_{_seq[0]}"
    p = os.path.join(ROOT, name)
    os.makedirs(p)
    cmds = [f"git init -q -b main {init_args} ."] + [f"git config {k} {v}" for k, v in cfg] + \
        [f"echo a{i} >> a; echo b{i} >> b; git add a b; git commit -qm c{i}" for i in range(commits)]
    for c in cmds:
        rc, o = h(name, c)
        if rc:
            raise SystemExit(f"setup failed in {name}: {c}: {o}")
    return name


def rev(repo, r="HEAD"):
    return h(repo, f"git rev-parse -q --verify {r}")[1]


def host_state(repo, branch="main"):
    """What the host sees after a trial: history, tree, fsck, reflog, status."""
    s = {}
    s["log"] = h(repo, f"git log --format='%h %p %s' {branch}")[1].splitlines()
    s["tree"] = h(repo, f"git ls-tree -r --name-only {branch}")[1].split()
    rc, out = h(repo, "git fsck --full --no-progress 2>&1")
    s["fsck_rc"], s["fsck"] = rc, out
    s["reflog"] = h(repo, f"git reflog --format='%h %gs' {branch}")[1].splitlines()
    s["status"] = h(repo, "git status --porcelain")[1].splitlines()
    return s


def is_anc(repo, a, b):
    return h(repo, f"git merge-base --is-ancestor {a} {b}")[0] == 0


def out(logf, rec):
    line = json.dumps(rec)
    print(line, flush=True)
    with open(logf, "a") as f:
        f.write(line + "\n")
