//! The rootless podman runtime adapter. Linux boxes share the host kernel,
//! so it adds the controls the spec's "podman adds" list names.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tokio::process::Child;

use crate::core::clean::LAYER_LABEL;
use crate::core::plan::{Env, Plan};
use crate::core::rfc3339;
use crate::core::runtime::{
    BoxInfo, BoxStat, BuildRequest, ImageInfo, MemoryStat, PidsStat, Runtime, bind, inspect,
    output, parse_json, spawn_error,
};
use crate::dirs;

/// Where the bind-mounted proxy socket appears in the box.
pub const GUEST_PROXY_SOCKET: &str = "/run/pinfold/proxy.sock";

/// The most tasks one box may create. The spec requires the cap, not a value;
/// 2048 leaves room for a toolchain's threads and still bounds a fork bomb.
const PIDS_LIMIT: u32 = 2048;

/// `CLONE_NEWUSER` from `linux/sched.h`.
const CLONE_NEWUSER: u64 = 0x1000_0000;

/// Rootless podman.
pub struct Podman;

impl Runtime for Podman {
    fn up(
        &self,
        plan: &Plan,
        init: &Path,
        proxy_socket: Option<&Path>,
        seccomp: Option<&Path>,
    ) -> io::Result<Child> {
        // Tighten the socket before anything slow, so it is not connectable
        // by another host user while the profile is derived: 0600 in its
        // 0700 state directory. The box user is the same uid under keep-id,
        // so it can still connect.
        if let Some(socket) = proxy_socket {
            let dir = socket
                .parent()
                .expect("the proxy socket lives in the box's state directory");
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        }
        let source = seccomp.expect("podman's preflight refuses a host with no seccomp profile");
        let seccomp = seccomp_profile(source)?;
        // --dns none cannot be combined with --network none on podman
        // 5.4.2; without this bind podman writes a resolver of its own.
        let cache = dirs::cache_dir()?;
        fs::create_dir_all(&cache)?;
        let resolv_conf = cache.join("resolv.conf");
        fs::write(&resolv_conf, "")?;
        let mut extra: Vec<OsString> = vec![
            "--userns=keep-id".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--security-opt".into(),
            format!("seccomp={}", seccomp.display()).into(),
            "--no-hosts".into(),
            "--mount".into(),
            bind(&resolv_conf, Path::new("/etc/resolv.conf"), true),
            "--pids-limit".into(),
            PIDS_LIMIT.to_string().into(),
            // podman's run copies the client's proxy variables in by default.
            "--http-proxy=false".into(),
        ];
        if let Some(memory) = &plan.memory {
            // Swap equal to memory: the limit is the memory the box gets, and
            // no swap beyond it.
            extra.push("--memory-swap".into());
            extra.push(memory.into());
        }
        // HOME is explicit: keep-id's injected passwd entry gives `/`, which
        // is read-only, so the box needs a real value. It goes on argv as a
        // literal and is not exported, because changing the podman client's
        // own HOME would move its rootless storage; HOME is a path, never a
        // secret.
        let mut home = OsString::from("HOME=");
        home.push(match plan.env.get("HOME") {
            Some(Env::Exact(value)) => OsString::from(value),
            Some(Env::From { from }) => std::env::var_os(from).unwrap_or_else(|| "/tmp".into()),
            None => "/tmp".into(),
        });
        extra.push("--env".into());
        extra.push(home);
        if let Some(socket) = proxy_socket {
            extra.push("--mount".into());
            extra.push(bind(socket, Path::new(GUEST_PROXY_SOCKET), true));
        }
        let guest_socket = proxy_socket.map(|_| GUEST_PROXY_SOCKET);
        let env = plan.env.iter().filter(|(name, _)| name.as_str() != "HOME");
        super::up("podman", plan, init, guest_socket, extra, env)?
            .spawn()
            .map_err(|error| spawn_error("podman", error))
    }

    fn preflight(&self) -> io::Result<Option<PathBuf>> {
        // A misconfigured host fails closed here, before any box starts.
        let json = output(&["podman", "info", "--format", "json"])?;
        let info: Info = parse_json("podman info", &json)?;
        if !info.host.security.rootless {
            return Err(io::Error::other(
                "rootful podman is not supported; pinfold requires rootless podman",
            ));
        }
        if info.host.cgroup_manager != "systemd" {
            return Err(io::Error::other(format!(
                "podman uses the {:?} cgroup manager; pinfold requires systemd, because cgroupfs silently ignores --cpus and --memory",
                info.host.cgroup_manager
            )));
        }
        if info.host.security.seccomp_profile_path.is_empty() {
            return Err(io::Error::other(
                "podman reports no seccomp profile; pinfold requires seccomp",
            ));
        }
        Ok(Some(PathBuf::from(info.host.security.seccomp_profile_path)))
    }

    fn down(&self, name: &str) -> io::Result<()> {
        // `-f` makes removing a box that is already gone succeed; `-t 0`
        // skips the stop grace period the spec does not grant.
        output(&["podman", "rm", "-f", "-t", "0", name]).map(drop)
    }

    fn stat(&self, name: &str) -> io::Result<BoxStat> {
        // The cgroup path comes from the runtime; pinfold never composes the
        // systemd scope name. Every field is a plain cgroup v2 file under it.
        let stdout = output(&[
            "podman",
            "inspect",
            "--format",
            "{{.State.CgroupPath}}",
            name,
        ])?;
        let cgroup = String::from_utf8_lossy(&stdout);
        let cgroup = cgroup.trim();
        if cgroup.is_empty() {
            return Err(io::Error::other(format!(
                "podman inspect {name} reports no cgroup path"
            )));
        }
        let dir = Path::new("/sys/fs/cgroup").join(cgroup.trim_start_matches('/'));
        Ok(BoxStat {
            name: name.to_string(),
            oom_kills: cgroup_event(&dir, "memory.events", "oom_kill"),
            memory: MemoryStat {
                current: cgroup_number(&dir, "memory.current"),
                peak: cgroup_number(&dir, "memory.peak"),
                limit: cgroup_number(&dir, "memory.max"),
            },
            pids: PidsStat {
                current: cgroup_number(&dir, "pids.current"),
                limit: cgroup_number(&dir, "pids.max"),
            },
        })
    }

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        let json = output(&["podman", "ps", "--all", "--format", "json"])?;
        let containers: Vec<ListedContainer> = parse_json("podman ps", &json)?;
        Ok(containers
            .into_iter()
            .map(|container| BoxInfo {
                // `down` and prune name the box, so its name is its id.
                id: container.names.into_iter().next().unwrap_or(container.id),
                labels: container.labels,
                image_id: container.image_id,
                image_ref: container.image,
                created: rfc3339(container.created),
                running: container.state == "running",
            })
            .collect())
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let json = output(&["podman", "image", "list", "--format", "json"])?;
        let images: Vec<ListedImage> = parse_json("podman image list", &json)?;
        let mut infos = Vec::new();
        // One entry per name, so Maintenance can remove every tag of an
        // old image; a dangling image is removed by its id. Podman emits
        // one JSON entry per tag and repeats the image's whole `Names` list
        // on each, so keep only the first entry per image.
        let mut seen = BTreeSet::new();
        for image in images {
            if !seen.insert(image.id.clone()) {
                continue;
            }
            let references = if image.names.is_empty() {
                vec![image.id.clone()]
            } else {
                image.names
            };
            for reference in references {
                infos.push(ImageInfo {
                    id: image.id.clone(),
                    reference,
                    labels: image.labels.clone(),
                    digest: None,
                });
            }
        }
        Ok(infos)
    }

    fn resolve_image(&self, reference: &str) -> io::Result<Result<ImageInfo, String>> {
        Ok(
            inspect::<InspectedImage>("podman", reference)?.map(|image| ImageInfo {
                id: image.id,
                reference: reference.to_string(),
                labels: image.labels,
                digest: Some(image.digest).filter(|digest| !digest.is_empty()),
            }),
        )
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image rm` also collects the layers no image references.
        output(&["podman", "image", "rm", reference]).map(|_| ())
    }

    fn purge_build_cache(&self) -> io::Result<()> {
        // `image prune` removes only dangling images and repeats until none
        // is left, so an intermediate goes once nothing builds on it, and an
        // intermediate under a kept image stays with it.
        let filter = format!("label={LAYER_LABEL}");
        output(&["podman", "image", "prune", "--force", "--filter", &filter]).map(|_| ())
    }

    fn build_cache(&self) -> &'static str {
        "the dev.pinfold.layer intermediate images no image builds on"
    }

    fn build(&self, request: &BuildRequest) -> io::Result<Result<(), String>> {
        let label = format!("{LAYER_LABEL}=true");
        let cache_flags: &[&str] = if request.cache {
            // The cache is intermediate images carrying only buildah's own
            // labels; this one is how `purge_build_cache` tells pinfold's
            // apart.
            &["--layer-label", &label]
        } else {
            // Every step reruns and no intermediate image is left behind.
            &["--layers=false"]
        };
        super::build("podman", cache_flags, request)
    }

    fn program(&self) -> &'static str {
        "podman"
    }

    fn name(&self) -> &'static str {
        "podman"
    }

    fn isolation(&self) -> &'static str {
        "the host kernel, shared with every box"
    }
}

/// One cgroup v2 file as a number, or `None` when the kernel does not have
/// the file (an old kernel's `memory.peak`) or the value is not a number
/// (`max`, meaning no limit).
fn cgroup_number(dir: &Path, file: &str) -> Option<u64> {
    fs::read_to_string(dir.join(file)).ok()?.trim().parse().ok()
}

/// One named counter from a cgroup v2 events file (`key value` lines). A
/// missing file is `None`.
fn cgroup_event(dir: &Path, file: &str, key: &str) -> Option<u64> {
    fs::read_to_string(dir.join(file))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(' ')?.parse().ok())
}

/// Host checks that do not depend on `podman info` succeeding. Doctor calls
/// these after a failed preflight; none changes the host.
pub fn host_requirements() -> Vec<String> {
    let mut missing = Vec::new();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", nix::unistd::getuid())));
    let runtime_dir = fs::metadata(&dir).and_then(|metadata| {
        if !metadata.is_dir() {
            return Err(io::Error::other("not a directory"));
        }
        nix::unistd::access(
            &dir,
            nix::unistd::AccessFlags::R_OK
                | nix::unistd::AccessFlags::W_OK
                | nix::unistd::AccessFlags::X_OK,
        )
        .map_err(io::Error::from)
    });
    if let Err(error) = runtime_dir {
        missing.push(format!(
            "runtime-dir: {}: {error}; rootless podman needs an accessible user runtime directory (XDG_RUNTIME_DIR or /run/user/<uid>)",
            dir.display()
        ));
    }
    if !Path::new("/run/systemd/system").is_dir() {
        missing.push(
            "systemd: /run/systemd/system is absent; pinfold requires the systemd cgroup manager and a systemd user session".to_string(),
        );
    }
    #[cfg(target_os = "linux")]
    match nix::sys::statfs::statfs("/sys/fs/cgroup") {
        Ok(stat) if stat.filesystem_type() == nix::sys::statfs::CGROUP2_SUPER_MAGIC => {}
        Ok(_) => missing.push(
            "cgroup-v2: /sys/fs/cgroup is not a cgroup2 filesystem; pinfold needs cgroup v2 for rootless resource limits".to_string(),
        ),
        Err(error) => missing.push(format!(
            "cgroup-v2: cannot inspect /sys/fs/cgroup: {error}"
        )),
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
    {
        missing.push(format!(
            "tun: /dev/net/tun: {error}; rootless podman networking needs access to the tun device"
        ));
    }
    missing
}

/// Whether `loginctl enable-linger` is on for this user, for `doctor`. A box
/// outlives the login that started it only with linger.
pub fn linger() -> io::Result<bool> {
    let uid = nix::unistd::getuid().to_string();
    let stdout = output(&[
        "loginctl",
        "show-user",
        &uid,
        "--property=Linger",
        "--value",
    ])?;
    Ok(String::from_utf8_lossy(&stdout).trim() == "yes")
}

/// What preflight reads from `podman info`.
#[derive(Debug, Deserialize)]
struct Info {
    host: InfoHost,
}

#[derive(Debug, Deserialize)]
struct InfoHost {
    /// `systemd`, `cgroupfs`, or another backend.
    #[serde(rename = "cgroupManager")]
    cgroup_manager: String,
    security: InfoSecurity,
}

#[derive(Debug, Deserialize)]
struct InfoSecurity {
    rootless: bool,
    #[serde(rename = "seccompProfilePath", default)]
    seccomp_profile_path: String,
}

/// Podman's default seccomp profile with nested user namespaces blocked.
///
/// The default allows `clone`, `clone3` and `unshare` unconditionally in one
/// large allow rule, so a conditional rule placed before it never runs.
/// Those names are removed from every allow rule, then `clone` and `unshare`
/// are allowed only when their flags do not include `CLONE_NEWUSER`. `clone3`
/// takes a pointer and its flags cannot be filtered, so it falls to the
/// profile's default action (`ENOSYS` in podman's profile) and callers fall
/// back to `clone`, where the flag is visible. `source` is the default
/// profile's path, as `podman info` reports it.
fn seccomp_profile(source: &Path) -> io::Result<PathBuf> {
    let what = format!("podman's seccomp profile {}", source.display());
    let text = fs::read(source)
        .map_err(|error| io::Error::new(error.kind(), format!("read {what}: {error}")))?;
    let mut profile: serde_json::Value = parse_json(&what, &text)?;
    let syscalls = profile
        .get_mut("syscalls")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| io::Error::other(format!("{what} has no syscalls list")))?;
    syscalls.retain_mut(|rule| {
        if rule.get("action").and_then(serde_json::Value::as_str) != Some("SCMP_ACT_ALLOW") {
            return true;
        }
        let Some(names) = rule
            .get_mut("names")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return true;
        };
        names.retain(|name| !matches!(name.as_str(), Some("clone" | "clone3" | "unshare")));
        // An allow rule whose names were all removed is dead; drop it.
        !names.is_empty()
    });
    for name in ["clone", "unshare"] {
        syscalls.push(serde_json::json!({
            "names": [name],
            "action": "SCMP_ACT_ALLOW",
            "args": [{
                "index": 0,
                "value": CLONE_NEWUSER,
                "valueTwo": 0,
                "op": "SCMP_CMP_MASKED_EQ",
            }],
        }));
        syscalls.push(serde_json::json!({
            "names": [name],
            "action": "SCMP_ACT_ERRNO",
            "errnoRet": 1,
            "args": [{
                "index": 0,
                "value": CLONE_NEWUSER,
                "valueTwo": CLONE_NEWUSER,
                "op": "SCMP_CMP_MASKED_EQ",
            }],
        }));
    }
    let dir = dirs::cache_dir()?.join("seccomp");
    fs::create_dir_all(&dir)?;
    let path = dir.join("podman.json");
    // Write beside and rename, so a concurrent `box up` never hands podman a
    // half-written profile.
    let temp = dir.join(format!(".podman-{}.json", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec(&profile).map_err(io::Error::other)?,
    )?;
    fs::rename(&temp, &path)?;
    Ok(path)
}

/// One `podman ps --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedContainer {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default, deserialize_with = "empty_default")]
    names: Vec<String>,
    #[serde(rename = "Labels", default, deserialize_with = "empty_default")]
    labels: BTreeMap<String, String>,
    #[serde(rename = "ImageID", default)]
    image_id: String,
    #[serde(rename = "Image", default)]
    image: String,
    /// Unix seconds.
    #[serde(rename = "Created")]
    created: i64,
    #[serde(rename = "State")]
    state: String,
}

/// One `podman image list --format json` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct ListedImage {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default, deserialize_with = "empty_default")]
    names: Vec<String>,
    #[serde(rename = "Labels", default, deserialize_with = "empty_default")]
    labels: BTreeMap<String, String>,
}

/// One `podman image inspect` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct InspectedImage {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Digest", default)]
    digest: String,
    #[serde(rename = "Labels", default, deserialize_with = "empty_default")]
    labels: BTreeMap<String, String>,
}

/// A field podman marshals as `null` when it is empty.
fn empty_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
