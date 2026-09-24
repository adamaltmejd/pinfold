//! The rootless podman runtime adapter.
//!
//! Linux boxes share the host kernel, so the adapter adds the controls the
//! spec names for podman: keep-id user namespaces, no-new-privileges, a
//! seccomp profile that blocks nested user namespaces, no host `/etc/hosts`
//! or resolvers, swap disabled, a task cap, and the proxy socket bind-mounted
//! 0600 in a 0700 state directory.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use serde::Deserialize;
use tokio::process::{Child, Command};

use crate::core::plan::{Env, Plan};
use crate::core::proxy::PROXY_URL;
use crate::core::runtime::{
    BoxInfo, BoxState, BuildRequest, ImageInfo, Runtime, bind, guest_path, spawn_error, user,
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
    fn up(&self, plan: &Plan, init: &Path, proxy_socket: Option<&Path>) -> io::Result<Child> {
        // Tighten the socket before anything slow, so it is not connectable
        // by another host user while preflight runs.
        if let Some(socket) = proxy_socket {
            tighten(socket)?;
        }
        let info = podman_info()?;
        let seccomp = seccomp_profile(&info)?;
        let resolv_conf = empty_resolv_conf()?;
        let argv = up_argv(plan, init, proxy_socket, &seccomp, &resolv_conf);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Values go through the child's environment, so argv holds only names
        // and `ps` cannot read a secret. HOME is the exception: it goes on
        // argv because changing the podman client's own HOME would move its
        // rootless storage.
        for (name, value) in &plan.env {
            if name == "HOME" {
                continue;
            }
            match value {
                Env::Exact(value) => {
                    command.env(name, value);
                }
                Env::From { from } => {
                    if let Some(value) = std::env::var_os(from) {
                        command.env(name, value);
                    }
                }
            }
        }
        // Always applied: Node's fetch reads the proxy variables only with
        // this set.
        command.env("NODE_USE_ENV_PROXY", "1");
        if plan.egress.is_some() {
            command.env("HTTPS_PROXY", PROXY_URL);
            command.env("http_proxy", PROXY_URL);
        }
        command
            .spawn()
            .map_err(|error| spawn_error("podman", error))
    }

    fn make_proxy_connectable(&self, _name: &str) -> io::Result<()> {
        // The bind-mounted socket already belongs to the box user under
        // keep-id; there is no root-owned forwarded copy to chmod.
        Ok(())
    }

    fn preflight(&self) -> io::Result<()> {
        preflight().map(|_| ())
    }

    fn down(&self, name: &str) -> io::Result<()> {
        let argv = down_argv(name);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let status = std::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .map_err(|error| spawn_error("podman", error))?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "podman rm -f -t 0 --ignore {name}: {status}"
            )))
        }
    }

    fn exec(
        &self,
        name: &str,
        tty: bool,
        workdir: Option<&Path>,
        argv: &[String],
    ) -> io::Result<ExitStatus> {
        let argv = exec_argv(name, tty, workdir, argv);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let mut command = std::process::Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if !tty {
            // Without a TTY the runtime's exec gets its own process group, so
            // a terminal signal reaches pinfold and not the exec.
            command.process_group(0);
        }
        command
            .status()
            .map_err(|error| spawn_error("podman", error))
    }

    fn list(&self) -> io::Result<Vec<BoxInfo>> {
        let output = std::process::Command::new("podman")
            .args(["ps", "--all", "--format", "json"])
            .output()
            .map_err(|error| spawn_error("podman", error))?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "podman ps: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_list(&output.stdout)
    }

    fn list_images(&self) -> io::Result<Vec<ImageInfo>> {
        let output = std::process::Command::new("podman")
            .args(["image", "list", "--format", "json"])
            .output()
            .map_err(|error| spawn_error("podman", error))?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "podman image list: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_images(&output.stdout)
    }

    fn remove_image(&self, reference: &str) -> io::Result<()> {
        // `image rm` also collects the layers no image references.
        let output = std::process::Command::new("podman")
            .args(["image", "rm", reference])
            .output()
            .map_err(|error| spawn_error("podman", error))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "podman image rm {reference}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn build(&self, request: &BuildRequest) -> io::Result<()> {
        let argv = build_argv(request);
        let (program, arguments) = argv.split_first().expect("argv is never empty");
        let status = std::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            // The build prints progress on stderr; pinfold prints the ref
            // itself, so the caller's stdout holds only the ref.
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| spawn_error("podman", error))?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("podman build: {status}")))
        }
    }

    fn image_digest(&self, reference: &str) -> io::Result<Option<String>> {
        // A floating tag must be pulled for its digest to be current and
        // present to inspect. `scratch` and other non-registry references
        // cannot be pulled; they simply have no digest.
        let _ = std::process::Command::new("podman")
            .args(["image", "pull", reference])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| spawn_error("podman", error));
        let output = std::process::Command::new("podman")
            .args(["image", "inspect", reference])
            .output()
            .map_err(|error| spawn_error("podman", error))?;
        if !output.status.success() {
            return Ok(None);
        }
        parse_digest(&output.stdout)
    }

    fn name(&self) -> &'static str {
        "podman"
    }

    fn isolation(&self) -> &'static str {
        "the host kernel, shared with every box"
    }

    fn version(&self) -> io::Result<String> {
        super::cli_version("podman")
    }
}

/// What `podman info` reports that `doctor` shows.
#[derive(Debug)]
pub struct PodmanInfo {
    /// `host.cgroupManager`: `systemd`, `cgroupfs`, or another backend.
    pub cgroup_manager: String,
    /// `host.idMappings.uidmap`: the container ids the user namespace maps.
    uid_map: Vec<IdMap>,
    /// `host.idMappings.gidmap`: the container ids the user namespace maps.
    gid_map: Vec<IdMap>,
}

/// One entry in podman's user-namespace id maps: container ids
/// `container_id` through `container_id + size - 1`.
#[derive(Debug, Deserialize)]
struct IdMap {
    container_id: u32,
    size: u32,
}

/// What `doctor` finds of podman on this host. Preflight refuses all but
/// rootless podman; `doctor` reports what it finds instead.
#[derive(Debug)]
pub enum Detected {
    RootlessPodman(PodmanInfo),
    RootfulPodman(PodmanInfo),
    Missing,
}

/// Ask the host what podman reports, for `doctor`. Reads only: a rootful,
/// cgroupfs or missing podman is an answer, not a failure.
pub fn detect() -> io::Result<Detected> {
    let info = match podman_info() {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Detected::Missing);
        }
        Err(error) => return Err(error),
    };
    let reported = PodmanInfo {
        cgroup_manager: info.host.cgroup_manager,
        uid_map: info.host.id_mappings.uidmap,
        gid_map: info.host.id_mappings.gidmap,
    };
    Ok(if info.host.security.rootless {
        Detected::RootlessPodman(reported)
    } else {
        Detected::RootfulPodman(reported)
    })
}

/// Whether `loginctl enable-linger` is on for this user, for `doctor`. A box
/// outlives the login that started it only with linger.
pub fn linger() -> io::Result<bool> {
    let uid = nix::unistd::getuid().to_string();
    let output = std::process::Command::new("loginctl")
        .args(["show-user", &uid, "--property=Linger"])
        .output()
        .map_err(|error| spawn_error("loginctl", error))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "loginctl show-user: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .find_map(|line| line.strip_prefix("Linger="))
        .is_some_and(|value| value == "yes"))
}

/// Whether `/dev/net/tun` exists, for `doctor`. pasta opens the device to
/// give a `RUN` step a network; a box itself runs with `--network none`, so
/// a missing device is a diagnosis, not a preflight refusal.
pub fn tun_present() -> bool {
    Path::new("/dev/net/tun").exists()
}

/// Whether both of podman's user-namespace id maps contain `id`, for
/// `doctor`. Debian's `_apt` is gid 65534 and cannot `setegid` to it when
/// the mapping stops short.
pub fn subordinate_ids_cover(info: &PodmanInfo, id: u32) -> bool {
    let covered = |map: &[IdMap]| {
        map.iter()
            .any(|entry| entry.container_id <= id && id - entry.container_id < entry.size)
    };
    covered(&info.uid_map) && covered(&info.gid_map)
}

/// What preflight needs from `podman info`.
#[derive(Deserialize)]
struct Info {
    host: InfoHost,
}

#[derive(Deserialize)]
struct InfoHost {
    #[serde(rename = "cgroupManager")]
    cgroup_manager: String,
    security: InfoSecurity,
    // Rootful podman reports no maps; rootless always does.
    #[serde(rename = "idMappings", default, deserialize_with = "empty_default")]
    id_mappings: InfoIdMappings,
}

#[derive(Deserialize, Default)]
struct InfoIdMappings {
    #[serde(default, deserialize_with = "empty_default")]
    uidmap: Vec<IdMap>,
    #[serde(default, deserialize_with = "empty_default")]
    gidmap: Vec<IdMap>,
}

#[derive(Deserialize)]
struct InfoSecurity {
    rootless: bool,
    #[serde(rename = "seccompProfilePath", default)]
    seccomp_profile_path: String,
}

/// Run and parse `podman info`. A missing CLI is named by the spawn helper,
/// so preflight and `doctor` say the same thing about it.
fn podman_info() -> io::Result<Info> {
    let output = std::process::Command::new("podman")
        .args(["info", "--format", "json"])
        .output()
        .map_err(|error| spawn_error("podman", error))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "podman info failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| io::Error::other(format!("podman info returned invalid JSON: {error}")))
}

/// Refuse a host pinfold will not run a box on, naming the problem. Runs
/// before any box starts; a misconfigured host fails closed here.
fn preflight() -> io::Result<Info> {
    let info = podman_info()?;
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
    Ok(info)
}

/// Whether the docker CLI is on `PATH`, for `doctor`'s second line: pinfold
/// never runs docker.
pub fn docker_present() -> bool {
    std::process::Command::new("docker")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Podman's default seccomp profile with nested user namespaces blocked.
///
/// The default allows `clone`, `clone3` and `unshare` unconditionally in one
/// large allow rule, so a conditional rule placed before it never runs.
/// Those names are removed from every allow rule, then `clone` and `unshare`
/// are allowed only when their flags do not include `CLONE_NEWUSER`. `clone3`
/// takes a pointer and its flags cannot be filtered, so it falls to the
/// profile's default action (`ENOSYS` in podman's profile) and callers fall
/// back to `clone`, where the flag is visible.
fn seccomp_profile(info: &Info) -> io::Result<PathBuf> {
    let source = Path::new(&info.host.security.seccomp_profile_path);
    let text = fs::read_to_string(source).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "read podman's seccomp profile {}: {error}",
                source.display()
            ),
        )
    })?;
    let mut profile: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        io::Error::other(format!(
            "podman's seccomp profile {} is not JSON: {error}",
            source.display()
        ))
    })?;
    let syscalls = profile
        .get_mut("syscalls")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| {
            io::Error::other(format!(
                "podman's seccomp profile {} has no syscalls list",
                source.display()
            ))
        })?;
    for rule in syscalls.iter_mut() {
        if rule.get("action").and_then(serde_json::Value::as_str) != Some("SCMP_ACT_ALLOW") {
            continue;
        }
        let Some(names) = rule
            .get_mut("names")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        names.retain(|name| !matches!(name.as_str(), Some("clone" | "clone3" | "unshare")));
    }
    // An allow rule whose names were all removed is dead; drop it.
    syscalls.retain(|rule| {
        rule.get("action").and_then(serde_json::Value::as_str) != Some("SCMP_ACT_ALLOW")
            || rule
                .get("names")
                .and_then(serde_json::Value::as_array)
                .is_none_or(|names| !names.is_empty())
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

/// The empty `/etc/resolv.conf` every box sees, owned by pinfold under the
/// cache dir and created once. `--dns none` cannot be combined with
/// `--network none` on podman 5.4.2; without this bind podman writes a
/// resolver of its own.
fn empty_resolv_conf() -> io::Result<PathBuf> {
    let path = dirs::cache_dir()?.join("resolv.conf");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(_) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(path),
        Err(error) => Err(error),
    }
}

/// Keep the proxy socket 0600 in its 0700 state directory. The box user is
/// the same uid under keep-id, so it can still connect.
fn tighten(socket: &Path) -> io::Result<()> {
    let dir = socket.parent().ok_or_else(|| {
        io::Error::other(format!(
            "proxy socket {} has no directory",
            socket.display()
        ))
    })?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(())
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
    /// Unix seconds.
    #[serde(rename = "Created")]
    created: i64,
    #[serde(rename = "State")]
    state: String,
}

fn parse_list(json: &[u8]) -> io::Result<Vec<BoxInfo>> {
    let containers: Vec<ListedContainer> = serde_json::from_slice(json)
        .map_err(|error| io::Error::other(format!("podman ps returned invalid JSON: {error}")))?;
    Ok(containers
        .into_iter()
        .map(|container| {
            let ListedContainer {
                id,
                names,
                labels,
                created,
                state,
            } = container;
            BoxInfo {
                // `down` and prune name the box, so its name is its id.
                id: names.into_iter().next().unwrap_or(id),
                labels,
                created: rfc3339(created),
                state: BoxState::from_runtime(&state),
            }
        })
        .collect())
}

/// The RFC 3339 UTC time for a Unix timestamp in seconds. Podman's
/// `Created` is Unix seconds; `box list` reports RFC 3339, like Apple.
fn rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (time / 3_600, time % 3_600 / 60, time % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a proleptic Gregorian date, after Howard
/// Hinnant's `civil_from_days` (public domain).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
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

fn parse_images(json: &[u8]) -> io::Result<Vec<ImageInfo>> {
    let images: Vec<ListedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!("podman image list returned invalid JSON: {error}"))
    })?;
    let mut infos = Vec::new();
    for image in images {
        let ListedImage { id, names, labels } = image;
        // One entry per name, so Maintenance can remove every tag of an old
        // image; a dangling image is removed by its id.
        if names.is_empty() {
            infos.push(ImageInfo {
                id: id.clone(),
                reference: id,
                labels,
            });
            continue;
        }
        for name in names {
            infos.push(ImageInfo {
                id: id.clone(),
                reference: local_reference(&name),
                labels: labels.clone(),
            });
        }
    }
    Ok(infos)
}

/// Podman prefixes locally built images with `localhost/`; the rest of
/// pinfold names them as they were tagged.
fn local_reference(name: &str) -> String {
    name.strip_prefix("localhost/").unwrap_or(name).to_string()
}

/// One `podman image inspect` entry, as much as pinfold needs.
#[derive(Deserialize)]
struct InspectedImage {
    #[serde(rename = "Digest", default)]
    digest: String,
}

fn parse_digest(json: &[u8]) -> io::Result<Option<String>> {
    let images: Vec<InspectedImage> = serde_json::from_slice(json).map_err(|error| {
        io::Error::other(format!(
            "podman image inspect returned invalid JSON: {error}"
        ))
    })?;
    Ok(images
        .into_iter()
        .next()
        .map(|image| image.digest)
        .filter(|digest| !digest.is_empty()))
}

/// A field podman marshals as `null` when it is empty.
fn empty_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// The `podman run` argv for a box, as data.
///
/// `init` is a host path; it and its directory are mounted read-only at the
/// same path, so it is also the path PID 1 runs. When `proxy_socket` is set,
/// the socket is bind-mounted read-only at [`GUEST_PROXY_SOCKET`] and init
/// relays to it as its argument.
pub fn up_argv(
    plan: &Plan,
    init: &Path,
    proxy_socket: Option<&Path>,
    seccomp: &Path,
    resolv_conf: &Path,
) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        "podman".into(),
        "run".into(),
        "-i".into(),
        "--name".into(),
        plan.name.clone().into(),
        "--network".into(),
        "none".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--user".into(),
        user(plan),
        "--userns=keep-id".into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--security-opt".into(),
        format!("seccomp={}", seccomp.display()).into(),
        "--no-hosts".into(),
        // `--dns none` conflicts with `--network none` on podman 5.4.2. An
        // empty read-only file leaves the box with no resolvers either way,
        // and podman's own generated resolv.conf never appears.
        "--mount".into(),
        bind(resolv_conf, Path::new("/etc/resolv.conf"), true),
        "--pids-limit".into(),
        PIDS_LIMIT.to_string().into(),
    ];
    if let Some(cpus) = plan.cpus {
        argv.push("--cpus".into());
        argv.push(cpus.to_string().into());
    }
    if let Some(memory) = &plan.memory {
        argv.push("--memory".into());
        argv.push(memory.into());
        // Swap equal to memory: the limit is the memory the box gets, and
        // no swap beyond it.
        argv.push("--memory-swap".into());
        argv.push(memory.into());
    }
    for (key, value) in &plan.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    for name in plan.env.keys().filter(|name| name.as_str() != "HOME") {
        // Names only: podman reads the value from our environment.
        argv.push("--env".into());
        argv.push(name.into());
    }
    // HOME is explicit: keep-id's injected passwd entry gives `/`, which is
    // read-only, so the box needs a real value. It goes on argv as a literal
    // because changing the podman client's own HOME would move its rootless
    // storage; HOME is a path, never a secret.
    argv.push("--env".into());
    let mut home = OsString::from("HOME=");
    home.push(home_value(plan));
    argv.push(home);
    // Always applied: Node's fetch reads the proxy variables only with this
    // set.
    argv.push("--env".into());
    argv.push("NODE_USE_ENV_PROXY".into());
    if plan.egress.is_some() {
        argv.push("--env".into());
        argv.push("HTTPS_PROXY".into());
        argv.push("--env".into());
        argv.push("http_proxy".into());
    }
    for mount in &plan.mounts {
        argv.push("--mount".into());
        argv.push(bind(&mount.host, &mount.guest, mount.readonly));
    }

    // The init binary comes from the host and runs as PID 1. box::up refuses
    // an init without a directory, so the parent is a real directory here.
    let init_dir = init.parent().unwrap_or(Path::new("/"));
    argv.push("--mount".into());
    argv.push(bind(init_dir, &guest_path(init_dir), true));
    if let Some(socket) = proxy_socket {
        argv.push("--mount".into());
        argv.push(bind(socket, Path::new(GUEST_PROXY_SOCKET), true));
    }
    argv.push("--entrypoint".into());
    argv.push(guest_path(init).into_os_string());
    argv.push(
        plan.image
            .clone()
            .expect("box up resolves the profile's image before the runtime runs")
            .into(),
    );
    argv.push("init".into());
    if proxy_socket.is_some() {
        argv.push(GUEST_PROXY_SOCKET.into());
    }
    argv
}

/// The HOME the box sees: the spec's, else `/tmp`, the one writable path.
fn home_value(plan: &Plan) -> OsString {
    match plan.env.get("HOME") {
        Some(Env::Exact(value)) => value.as_str().into(),
        Some(Env::From { from }) => std::env::var_os(from).unwrap_or_else(|| "/tmp".into()),
        None => "/tmp".into(),
    }
}

/// The `podman build` argv for one build, as data.
pub fn build_argv(request: &BuildRequest) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![
        "podman".into(),
        "build".into(),
        // Every build reruns every step and leaves no intermediate images
        // behind, so the default `purge_build_cache` is true of this
        // adapter.
        "--layers=false".into(),
        "--file".into(),
        request.containerfile.into(),
    ];
    for tag in request.tags {
        argv.push("--tag".into());
        argv.push(tag.into());
    }
    for (key, value) in request.labels {
        argv.push("--label".into());
        argv.push(format!("{key}={value}").into());
    }
    argv.push(request.context.into());
    argv
}

/// The `podman rm` argv that stops and removes a box, as data. `--ignore`
/// makes removing a box that is already gone succeed.
pub fn down_argv(name: &str) -> Vec<OsString> {
    vec![
        "podman".into(),
        "rm".into(),
        "-f".into(),
        "-t".into(),
        "0".into(),
        "--ignore".into(),
        name.into(),
    ]
}

/// The `podman exec` argv, as data.
///
/// The process inherits the run's user, so box-created files stay the host
/// user's. With a TTY, keep the host's terminal identity: the runtime
/// otherwise reports its own `TERM`.
pub fn exec_argv(name: &str, tty: bool, workdir: Option<&Path>, argv: &[String]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["podman".into(), "exec".into(), "-i".into()];
    if tty {
        args.push("-t".into());
        for variable in ["TERM", "COLORTERM"] {
            if let Ok(value) = std::env::var(variable) {
                args.push("--env".into());
                args.push(format!("{variable}={value}").into());
            }
        }
    }
    if let Some(dir) = workdir {
        args.push("--workdir".into());
        args.push(dir.as_os_str().to_os_string());
    }
    args.push(name.into());
    for arg in argv {
        args.push(OsString::from(arg));
    }
    args
}
