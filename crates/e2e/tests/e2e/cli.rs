//! End-to-end tests for host CLI options, diagnostics and self-update.
//!
//! The options need no runtime. Doctor is exercised with the host runtime
//! available and absent from PATH.

use std::fs;
use std::path::{Path, PathBuf};

use e2e::{TestEnv, pinfold};

#[path = "update.rs"]
mod update;

/// A fresh host with no runtime on PATH: `pinfold --version` and `pinfold
/// box list --help` answer from the binary alone, spawn nothing and leave the
/// state dir untouched.
#[test]
fn version_needs_no_runtime() {
    let _runtime = crate::shared_runtime();
    // Sabotage: run `crate::core::clean::maintain()` first in main.rs,
    // before the options and the help check, as the code once did; with the
    // empty PATH the pass writes under the state dir, so both empty-state
    // assertions fail. Sabotage: print `pinfold 0.0.0` in main.rs's
    // `--version` arm instead of `CARGO_PKG_VERSION`; stdout lacks the
    // workspace version and the version assertion fails. Sabotage: in
    // main.rs, answer `--help` after the daily pass instead of before it;
    // `box list --help` then writes the maintenance state first, and its
    // empty-state assertion fails.
    let env = TestEnv::new("version");
    let empty = env.root.join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    // TestEnv creates the state dir, so untouched means still empty.
    let state_entries = || {
        fs::read_dir(&env.state)
            .expect("read the state dir")
            .count()
    };

    let output = env
        .command(pinfold())
        .arg("--version")
        .env("PATH", &empty)
        .output()
        .expect("run pinfold --version");

    assert!(
        output.status.success(),
        "pinfold --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The e2e crate shares the workspace version with pinfold.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "pinfold --version printed no version: {stdout}"
    );
    assert_eq!(state_entries(), 0, "pinfold --version wrote state");

    let help = env
        .command(pinfold())
        .args(["box", "list", "--help"])
        .env("PATH", &empty)
        .output()
        .expect("run pinfold box list --help");
    assert!(
        help.status.success(),
        "pinfold box list --help failed: {}",
        String::from_utf8_lossy(&help.stderr)
    );
    assert_eq!(state_entries(), 0, "pinfold box list --help wrote state");
}

#[test]
fn doctor_reports_without_changing_state() {
    let _runtime = crate::shared_runtime();
    // Guarantee 27. Sabotage: run maintenance before doctor; the fresh
    // state directory gains a maintenance stamp and the old artifact goes.
    // Sabotage: omit host_requirements after a failed preflight; the Linux
    // refusal no longer names the missing runtime directory. Drop the
    // ioctl from the kvm check; the device the Landlock domain leaves
    // unusable loses its `kvm` line. Check membership of the kvm group
    // instead of access; CI's user is outside it, its 0666 device is
    // usable, and the profiled binary reports `kvm`. Choose the
    // user-namespace token from the probe's error; with AppArmor's
    // restriction on, the unprofiled copy reports `userns`. Skip the
    // probe; the copy reports no user-namespace token.
    // Materialize an embedded profile during inspection; the fresh cache
    // gains files. Expected trees are the host's own pre-inspection snapshot.
    // Podman's image lookup may create its own short-name alias lock outside
    // these owned trees. Missing directories differ from empty directories.
    let env = TestEnv::with_private_cache("doctor");
    fs::write(env.root.join(".pinfold.toml"), "memory = \"1G\"\n").unwrap();
    let old = env.root.join("cache/pinfold/artifacts/pi/0.0.0/marker");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, b"keep").unwrap();
    let owned = [
        env.state.join("pinfold"),
        env.config.join("pinfold"),
        env.root.join("cache/pinfold"),
    ];
    let owned_trees = || {
        owned
            .iter()
            .map(|directory| {
                let tree = match fs::symlink_metadata(directory) {
                    Ok(_) => Some(host_tree(directory)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => panic!("read {}: {error}", directory.display()),
                };
                (directory.clone(), tree)
            })
            .collect::<Vec<_>>()
    };
    let before = owned_trees();
    let empty = env.root.join("empty-path");
    fs::create_dir(&empty).unwrap();
    let missing = env.root.join("missing-runtime");

    // An executable outside the AppArmor profile's path. CI profiles only
    // the built binary.
    let unprofiled = env.root.join("unprofiled/pinfold");
    fs::create_dir_all(unprofiled.parent().unwrap()).unwrap();
    fs::copy(pinfold(), &unprofiled).unwrap();
    // From the host, not from pinfold: the kernel knob AppArmor's
    // restriction is.
    let apparmor_restricts =
        fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|value| value.trim() == "1");

    // The real Podman also fails before reporting its host settings when
    // XDG_RUNTIME_DIR is absent, so every scenario but `ready` lists the
    // host requirements. All reports leave the same fixtures alone.
    for scenario in [
        "ready",
        "no-runtime",
        "missing-runtime-dir",
        "no-kvm",
        "unprofiled",
    ] {
        let linux_only = matches!(scenario, "missing-runtime-dir" | "no-kvm" | "unprofiled");
        if linux_only && !cfg!(target_os = "linux") {
            continue;
        }
        let mut command = env.command(if scenario == "unprofiled" {
            &unprofiled
        } else {
            pinfold()
        });
        #[cfg(target_os = "linux")]
        if scenario == "no-kvm" {
            ioctl_dev_denied(&mut command);
        }
        command.arg("doctor").current_dir(&env.root);
        if scenario != "ready" {
            command.env("XDG_RUNTIME_DIR", &missing);
        }
        if scenario == "no-runtime" {
            command.env("PATH", &empty);
        }
        let output = command.output().expect("run pinfold doctor");
        assert!(output.status.success(), "doctor failed: {output:?}");
        // Sabotage: omit config_report after a failed runtime check. Host
        // configuration must still be readable; the fixture supplies 1G.
        let report = String::from_utf8_lossy(&output.stdout);
        let config: serde_json::Value = report
            .lines()
            .find_map(|line| {
                line.strip_prefix("config: ")
                    .and_then(|json| serde_json::from_str(json).ok())
            })
            .expect("doctor reports host configuration");
        assert_eq!(config["memory"], "1G");
        if scenario == "no-runtime" {
            assert!(config["image_built"].is_null());
            assert!(
                config["image_error"]
                    .as_str()
                    .is_some_and(|error| !error.is_empty())
            );
        }
        assert_eq!(
            owned_trees(),
            before,
            "doctor changed pinfold's state, config or cache ({scenario})"
        );
        if scenario != "ready" && cfg!(target_os = "linux") {
            let report = String::from_utf8_lossy(&output.stdout);
            assert!(
                report.contains("runtime-dir") && report.contains(missing.to_str().unwrap()),
                "doctor did not identify the missing runtime directory: {report}"
            );
            let tokens: Vec<&str> = report
                .lines()
                .filter_map(|line| line.trim_start().split_once(':').map(|(token, _)| token))
                .collect();
            let has = |token: &str| tokens.contains(&token);
            match scenario {
                "missing-runtime-dir" => assert!(
                    !has("kvm") && !has("userns") && !has("userns-apparmor"),
                    "the profiled binary on a usable host reported a jail requirement: {report}"
                ),
                "no-kvm" => assert!(
                    has("kvm"),
                    "doctor did not identify the unusable /dev/kvm: {report}"
                ),
                "unprofiled" if apparmor_restricts => assert!(
                    has("userns-apparmor") && !has("userns"),
                    "the unprofiled copy did not report userns-apparmor: {report}"
                ),
                "unprofiled" => assert!(
                    !has("userns") && !has("userns-apparmor"),
                    "without AppArmor's restriction the copy reported a user-namespace token: {report}"
                ),
                _ => {}
            }
        }
    }

    // The box half: the copy's `up` is refused as `runtime` naming the
    // token, and the profiled executable starts a box. Sabotage: skip the
    // jail preflight; the copy's `up` fails later, without the token.
    #[cfg(target_os = "linux")]
    if apparmor_restricts {
        use std::io::Write;
        use std::process::Stdio;
        let name = crate::box_::box_name("doctor");
        let spec = serde_json::json!({ "name": name, "image": e2e::default_image(&env) });
        let mut up = env
            .command(&unprofiled)
            .args(["box", "up"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn the copy's box up");
        up.stdin
            .take()
            .unwrap()
            .write_all(spec.to_string().as_bytes())
            .unwrap();
        let output = up.wait_with_output().unwrap();
        assert!(
            !output.status.success(),
            "the unprofiled copy started a box"
        );
        let refused: serde_json::Value = serde_json::from_slice(
            output
                .stdout
                .split(|&byte| byte == b'\n')
                .next()
                .unwrap_or_default(),
        )
        .expect("the copy's refusal is a JSON line");
        assert_eq!(refused["reason"], "runtime", "{refused}");
        assert!(
            refused["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("userns-apparmor")),
            "the copy's refusal does not name userns-apparmor: {refused}"
        );
        crate::box_::box_up(&env, &spec, &name).down(&env);
    }
}

/// Make `/dev/kvm` unusable to `command` with a real kernel refusal: a
/// Landlock domain that handles only device ioctls and allows none.
/// Opening the device still works; `KVM_GET_API_VERSION` gets EACCES.
#[cfg(target_os = "linux")]
fn ioctl_dev_denied(command: &mut std::process::Command) {
    use nix::libc;
    use std::os::unix::process::CommandExt;
    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }
    const LANDLOCK_ACCESS_FS_IOCTL_DEV: u64 = 1 << 15;
    // SAFETY: the hook makes only system calls on stack values.
    unsafe {
        command.pre_exec(|| {
            let attr = RulesetAttr {
                handled_access_fs: LANDLOCK_ACCESS_FS_IOCTL_DEV,
            };
            let ruleset = libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            );
            if ruleset < 0
                || libc::prctl(
                    libc::PR_SET_NO_NEW_PRIVS,
                    1 as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                    0 as libc::c_ulong,
                ) != 0
                || libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(ruleset as i32);
            Ok(())
        });
    }
}

// Preserve directories as well as file bytes: a supposedly read-only
// inspection must not create even an empty cache directory.
pub(super) fn host_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn walk(root: &Path, dir: &Path, entries: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if entry.file_type().unwrap().is_dir() {
                entries.push((relative, None));
                walk(root, &path, entries);
            } else {
                entries.push((relative, Some(fs::read(&path).unwrap())));
            }
        }
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort();
    entries
}
