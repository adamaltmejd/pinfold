//! Guarantee 43: the VMM reaches only its own box. Linux only.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use e2e::{Below, Leftover, Project, TestEnv, box_name, default_image, json_lines, run};
use serde_json::{Value, json};

const COMMAND: Duration = Duration::from_secs(120);
const NAMESPACES: [&str; 6] = ["user", "mnt", "pid", "net", "ipc", "uts"];

#[test]
fn the_vmm_reaches_only_its_own_box() {
    // Sabotage (run): drop `CLONE_NEWIPC` from the clone flags in
    // core/runtime/firecracker/jail.rs. Each VMM then shares the host's
    // IPC namespace.
    // Each VMM seeing its own disks, and the detached owner's SIGKILL
    // ending its VMM, are the positive controls.
    let image = default_image();
    let env = TestEnv::new("jail");

    // Box A: `box up` with one disk; its owner is the `up` process.
    let disk = format!("e2e-{}-jail", std::process::id());
    let a = env.up(&json!({
        "name": box_name("jail"),
        "image": image,
        "disks": [{ "name": disk, "guest": "/data", "size": "1G" }],
    }));
    let a_disks = vec![disk_path(&env, &disk)];

    // Box B: a disk project kept by `start`, whose owner is detached.
    let project = Project::new(&env, "jail");
    let started = project.pinfold(&["start"]);
    assert_eq!(started.code, 0, "start: {started:?}");
    let config: Value =
        serde_json::from_str(&project.pinfold(&["config", "--json"]).stdout).expect("config JSON");
    let id = config["project"]["id"].as_str().expect("project id");
    let b_disks: Vec<PathBuf> = ["home", "project"]
        .iter()
        .map(|kind| {
            disk_path(
                &env,
                config["project"]["disks"][kind]
                    .as_str()
                    .expect("disk name"),
            )
        })
        .collect();
    let label = format!("dev.pinfold.project={id}");
    let listed = run(
        env.command().args(["box", "list", "--label", &label]),
        COMMAND,
    );
    let listed = json_lines(&listed.stdout);
    assert_eq!(listed.len(), 1, "boxes of project {id}: {listed:?}");
    let leftover = Leftover::listed(&env, &listed[0]);

    let a_vmm = Vmm::of(a.pid());
    let b_below = Below::of(leftover.owner());
    let b_vmm = Vmm::of(leftover.owner());

    let uid = fs::metadata("/proc/self").unwrap().uid().to_string();
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let owners = [a.pid(), leftover.owner()];
    for (vmm, own, other) in [(&a_vmm, &a_disks, &b_disks), (&b_vmm, &b_disks, &a_disks)] {
        // The user's, with no capabilities and no way to gain privilege.
        let status = vmm.status();
        for field in ["Uid", "Gid"] {
            let ids: Vec<&str> = status[field].split_whitespace().collect();
            let expected = match field {
                "Uid" => uid.clone(),
                _ => fs::metadata("/proc/self").unwrap().gid().to_string(),
            };
            assert_eq!(ids, [expected.as_str(); 4], "{field}: {status:?}");
        }
        assert_eq!(status["CapBnd"], "0000000000000000", "{status:?}");
        assert_eq!(status["NoNewPrivs"], "1", "{status:?}");

        // Namespaces of its own: not the host's, not the other VMM's.
        let theirs = if vmm.pid == a_vmm.pid { &b_vmm } else { &a_vmm };
        for ns in NAMESPACES {
            let mine = namespace(vmm.pid, ns);
            assert_ne!(mine, namespace(std::process::id(), ns), "{ns}");
            assert_ne!(mine, namespace(theirs.pid, ns), "{ns}");
        }

        // Through its root: its own disks and not the other box's.
        let seen = vmm.tree();
        let inodes: BTreeSet<(u64, u64)> = seen.iter().map(|entry| entry.inode).collect();
        for path in own {
            assert!(
                inodes.contains(&inode(path)),
                "{} not in the jail",
                path.display()
            );
        }
        for path in other {
            assert!(
                !inodes.contains(&inode(path)),
                "{} in the jail",
                path.display()
            );
        }

        // No mount point is under the home.
        let mountinfo = fs::read_to_string(format!("/proc/{}/mountinfo", vmm.pid)).unwrap();
        for line in mountinfo.lines() {
            let point = line.split(' ').nth(4).expect("a mount point");
            assert!(!Path::new(point).starts_with(&home), "{line}");
        }

        // Every socket in the jail is Firecracker's: its vsock socket or a
        // host listener for a guest port, `<socket>_<port>` (Firecracker's
        // vsock convention), in the socket's directory.
        let config: Value =
            serde_json::from_slice(&fs::read(vmm.root().join("config.json")).unwrap())
                .expect("the VM configuration");
        let uds = Path::new(config["vsock"]["uds_path"].as_str().expect("uds_path"));
        let (dir, base) = (
            uds.parent().unwrap(),
            uds.file_name().unwrap().to_str().unwrap(),
        );
        let vsock_name = |name: &str| {
            name == base
                || name
                    .strip_prefix(base)
                    .and_then(|rest| rest.strip_prefix('_'))
                    .is_some_and(|port| {
                        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                    })
        };
        for entry in seen.iter().filter(|entry| entry.socket) {
            let name = entry.path.file_name().and_then(|n| n.to_str());
            assert!(
                entry.path.parent() == Some(dir) && name.is_some_and(vsock_name),
                "a socket in the jail: {}",
                entry.path.display()
            );
        }
        // Seen from the host: the owners' other listening sockets, the
        // project's session and shutdown control among them, have no
        // entry in the jail. pinfold binds them by name in their
        // directory, so /proc names only their leaf.
        for owner in owners {
            let controls: Vec<String> = listening_sockets(owner)
                .into_iter()
                .filter(|name| !vsock_name(name))
                .collect();
            if owner == leftover.owner() {
                assert!(
                    controls.len() >= 2,
                    "the project owner's control sockets: {controls:?}"
                );
            }
            for entry in &seen {
                let name = entry.path.file_name().and_then(|n| n.to_str());
                assert!(
                    !name.is_some_and(|name| controls.iter().any(|c| c == name)),
                    "{} in the jail",
                    entry.path.display()
                );
            }
        }
    }

    // Control: SIGKILL of the detached owner ends its VM.
    let kill = run(
        Command::new("kill").args(["-KILL", &leftover.owner().to_string()]),
        COMMAND,
    );
    assert_eq!(kill.code, 0, "kill the project owner: {kill:?}");
    b_below.wait_gone();
    drop(leftover);
    drop(project);
    drop(a);
}

/// A Linux disk's backing file: `<state>/pinfold/disks/<name>.ext4` (Disks).
fn disk_path(env: &TestEnv, name: &str) -> PathBuf {
    let path = env.state_home().join(format!("pinfold/disks/{name}.ext4"));
    assert!(path.is_file(), "{}", path.display());
    path
}

fn inode(path: &Path) -> (u64, u64) {
    let meta = fs::metadata(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (meta.dev(), meta.ino())
}

fn namespace(pid: u32, ns: &str) -> PathBuf {
    fs::read_link(format!("/proc/{pid}/ns/{ns}")).unwrap_or_else(|e| panic!("{pid} {ns}: {e}"))
}

/// One VMM process, found as the one `firecracker` below its owner.
struct Vmm {
    pid: u32,
}

struct Entry {
    /// The path as the VMM sees it, from its root.
    path: PathBuf,
    inode: (u64, u64),
    socket: bool,
}

impl Vmm {
    fn of(owner: u32) -> Vmm {
        let found = Below::of(owner).named("firecracker");
        assert_eq!(found.len(), 1, "VMMs below {owner}: {found:?}");
        Vmm { pid: found[0] }
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(format!("/proc/{}/root", self.pid))
    }

    /// `/proc/PID/status` fields.
    fn status(&self) -> std::collections::BTreeMap<String, String> {
        fs::read_to_string(format!("/proc/{}/status", self.pid))
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_string(), value.trim().to_string()))
            .collect()
    }

    /// Every entry under the VMM's root, without following symlinks.
    fn tree(&self) -> Vec<Entry> {
        let root = self.root();
        let mut found = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    dirs.push(path.clone());
                }
                found.push(Entry {
                    path: Path::new("/").join(path.strip_prefix(&root).unwrap()),
                    inode: (meta.dev(), meta.ino()),
                    socket: meta.file_type().is_socket(),
                });
            }
        }
        assert!(!found.is_empty(), "the VMM's root is empty");
        found
    }
}

/// The names of the Unix sockets `pid` listens on, from /proc: its fds'
/// socket inodes, named in its network namespace's socket table.
fn listening_sockets(pid: u32) -> Vec<String> {
    let held: BTreeSet<String> = fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
        .filter_map(|link| {
            link.to_str()?
                .strip_prefix("socket:[")?
                .strip_suffix(']')
                .map(str::to_string)
        })
        .collect();
    // `Num RefCount Protocol Flags Type St Inode Path`; Flags 00010000 is
    // a listening socket.
    fs::read_to_string(format!("/proc/{pid}/net/unix"))
        .unwrap()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let path = fields.get(7)?;
            (fields[3] == "00010000" && held.contains(fields[6]))
                .then(|| path.rsplit('/').next().unwrap_or(path).to_string())
        })
        .collect()
}
