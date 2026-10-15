#!/usr/bin/env python3
"""Temporary issue #92 ABBA diagnostic. Run from the repository root."""

import hashlib
import json
import os
import platform
import signal
import subprocess
import sys
import time
from pathlib import Path


BASE = "36adf1495439f8e3f2714a546af51cdb1e846056"
LIB = Path("crates/e2e/src/lib.rs")
A_SHA256 = "2bb7c24d929542027952c6fbbad666a9bd62ad95e7080e5652f888e23c22bf89"
B_SHA256 = "096e347180224187ddeb379007b0e8abf854e2612a09b786a76df5539a59293f"
ROOT = Path("target/issue92-mutex-abba").resolve()
ORDER = ("A", "B", "B", "A")
ALLOWED_COMMIT_PATHS = {
    ".github/workflows/ci.yml",
    "docs/archive/2026-10-15-issue-92-build-mutex-test-audit.md",
    "scripts/issue92-mutex-abba.py",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(("git", *args))


def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def assert_source(expected):
    assert LIB.read_bytes() == expected, "lib.rs differs from reviewed variant"
    dirty = set(git("diff", "--name-only", "-z").decode().split("\0")) - {""}
    assert dirty <= {str(LIB)}, f"unexpected tracked edits: {sorted(dirty)}"
    assert not git("diff", "--cached", "--name-only", "-z"), "staged edits present"


def terminate(proc):
    if proc is None or proc.poll() is not None:
        return
    os.killpg(proc.pid, signal.SIGTERM)
    try:
        proc.wait(timeout=20)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait()


def interrupted(signum, _frame):
    raise KeyboardInterrupt(f"signal {signum}")


def command(argv, cwd, logfile, resultfile, timefile=None):
    start = time.monotonic_ns()
    proc = None
    result = {"status": "not-run", "argv_kind": "suite" if argv[0] == "cargo" else "g35"}
    try:
        with logfile.open("wb") as output:
            actual = (("/usr/bin/time", "-f",
                       "elapsed=%e user=%U system=%S max_rss_kib=%M exit=%x",
                       "-o", str(timefile), *argv) if timefile else argv)
            proc = subprocess.Popen(actual, cwd=cwd, stdin=subprocess.DEVNULL,
                                    stdout=output, stderr=subprocess.STDOUT,
                                    start_new_session=True)
            rc = proc.wait()
        result = {"status": "complete", "exit": rc,
                  "elapsed_ns": time.monotonic_ns() - start,
                  "argv_kind": result["argv_kind"]}
    except BaseException as exc:
        result = {"status": "interrupted", "error_type": type(exc).__name__,
                  "elapsed_ns": time.monotonic_ns() - start,
                  "argv_kind": result["argv_kind"]}
        raise
    finally:
        terminate(proc)
        write_json(resultfile, result)
    return result["exit"]


def observed_suite(directory):
    observer = None
    suite_rc = None
    with (directory / "host.jsonl").open("wb") as output, \
         (directory / "observer.stderr").open("wb") as errors:
        try:
            observer = subprocess.Popen((sys.executable, "scripts/diagnose-suite-host.py"),
                                        stdin=subprocess.DEVNULL, stdout=output,
                                        stderr=errors, start_new_session=True)
            suite_rc = command(("cargo", "test", "-p", "e2e", "--locked", "--", "--nocapture"),
                               None, directory / "suite.log", directory / "suite-result.json",
                               directory / "command-time.txt")
        finally:
            terminate(observer)
            write_json(directory / "observer-result.json", {
                "status": "complete" if observer is not None else "not-run",
                "exit": observer.returncode if observer is not None else None,
            })
    return suite_rc, observer.returncode


def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    for number, variant in enumerate(ORDER, 1):
        directory = ROOT / f"{number}-{variant}"
        directory.mkdir(exist_ok=True)
        for name in ("suite-result.json", "g35-result.json", "observer-result.json"):
            write_json(directory / name, {"status": "not-run"})

    original = LIB.read_bytes()
    a = git("show", f"{BASE}:{LIB}")
    needle = b"pub fn build_profile(env: &TestEnv, name: &str) {\n"
    assert a.count(needle) == 1, "expected one helper entry"
    b = a.replace(needle, needle + (
        b'    #[cfg(target_os = "linux")]\n'
        b'    static BUILDS: std::sync::Mutex<()> = std::sync::Mutex::new(());\n'
        b'    #[cfg(target_os = "linux")]\n'
        b'    let _build = BUILDS.lock().unwrap_or_else(|poison| poison.into_inner());\n'
    ))
    source_valid = False
    try:
        assert original == a, "checkout must start at reviewed A bytes"
        source_valid = True
        assert sha(a) == A_SHA256, "A bytes differ from reviewed base"
        assert sha(b) == B_SHA256, "B bytes differ from reviewed variant"
        assert a != b, "A and B variants match"
        changed = set(git("diff", "--name-only", "-z", BASE, "HEAD").decode().split("\0")) - {""}
        assert changed <= ALLOWED_COMMIT_PATHS, f"unexpected committed change: {sorted(changed - ALLOWED_COMMIT_PATHS)}"
        assert not git("ls-files", "--others", "--exclude-standard", "-z"), "untracked source files"
        assert not any(os.environ.get(name) for name in
                       ("RUST_TEST_THREADS", "CARGO_BUILD_JOBS", "CARGO_TARGET_DIR")), "worker/target override"
        assert len(os.sched_getaffinity(0)) == 4, "expected ordinary four-CPU affinity"
        assert platform.machine() == "x86_64", "x64 host required"
        write_json(ROOT / "source.json", {
            "base": BASE, "head": git("rev-parse", "HEAD").decode().strip(),
            "A_sha256": sha(a), "B_sha256": sha(b), "order": ORDER,
            "affinity_cpus": sorted(os.sched_getaffinity(0)),
            "comparison": "B1/B2 versus final A2; initial A includes initial-store effects",
        })
        failed = False
        binary = Path.cwd() / "target/x86_64-unknown-linux-musl/debug/pinfold"
        for number, variant in enumerate(ORDER, 1):
            directory = ROOT / f"{number}-{variant}"
            expected = a if variant == "A" else b
            if LIB.read_bytes() != expected:
                LIB.write_bytes(expected)
            assert_source(expected)
            write_json(directory / "source.json", {"variant": variant, "lib_sha256": sha(LIB.read_bytes())})
            suite_rc, observer_rc = observed_suite(directory)
            failed |= suite_rc != 0 or observer_rc != 0
            write_json(directory / "binary.json", {
                "status": "present" if binary.is_file() else "absent",
                "sha256": sha(binary.read_bytes()) if binary.is_file() else None,
            })
            if binary.is_file():
                g35 = ("bash", "-c", 'bun install --frozen-lockfile && bun run check && '
                       'bun durable_recovery_preserves_boxed_execution.ts '
                       '--pinfold "$1" --image pinfold/profile-default:latest',
                       "issue92-g35", str(binary))
                g35_rc = command(g35, "examples/pi-durable", directory / "g35.log",
                                 directory / "g35-result.json")
                failed |= g35_rc != 0
            else:
                write_json(directory / "g35-result.json", {"status": "not-run", "reason": "binary absent"})
                failed = True
            assert_source(expected)
            print(json.dumps({"run": number, "variant": variant,
                              "suite_exit": suite_rc, "observer_exit": observer_rc,
                              "suite": json.loads((directory / "suite-result.json").read_text()),
                              "g35": json.loads((directory / "g35-result.json").read_text())}), flush=True)
        return int(failed)
    finally:
        restore = a if source_valid else original
        if LIB.read_bytes() != restore:
            LIB.write_bytes(restore)
        write_json(ROOT / "restoration.json", {"restored_A": LIB.read_bytes() == a,
                                                "lib_sha256": sha(LIB.read_bytes())})


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        exit_code = main()
    except BaseException as exc:
        ROOT.mkdir(parents=True, exist_ok=True)
        write_json(ROOT / "fatal.json", {"error_type": type(exc).__name__, "message": str(exc)})
        raise

    sys.exit(exit_code)
