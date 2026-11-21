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
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    // pinfold's state, the disks among it, lives under the home by default.
    // The mount check below only means something when it does here too.
    assert!(
        env.state_home().starts_with(&home),
        "the state root {} is not under $HOME: set TMPDIR to a directory \
         under the home and outside any Git checkout",
        env.state_home().display()
    );

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
    // Every socket file under the test's state root and the runtime
    // directory, `/tmp/pinfold-<uid>` (Ownership), with the identity of
    // its directory.
    let roots = [
        env.state_home().to_path_buf(),
        PathBuf::from(format!("/tmp/pinfold-{uid}")),
    ];
    let host_sockets = sockets_under(&roots);
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

        // No directory from the home's filesystem is mounted in the jail:
        // each mount point on the home's device is a single file, and the
        // box's own disks are among them. The jail's root is a tmpfs, on
        // a device of its own, so it is not exempted.
        let home_dev = fs::metadata(&home).unwrap().dev();
        let mountinfo = fs::read_to_string(format!("/proc/{}/mountinfo", vmm.pid)).unwrap();
        let mut from_home = BTreeSet::new();
        for line in mountinfo.lines() {
            let point = unescape(line.split(' ').nth(4).expect("a mount point"));
            let meta = fs::metadata(vmm.root().join(point.trim_start_matches('/')))
                .unwrap_or_else(|e| panic!("{point}: {e}"));
            if meta.dev() == home_dev {
                assert!(
                    meta.is_file(),
                    "a mount from the home that is not a file: {line}"
                );
                from_home.insert((meta.dev(), meta.ino()));
            }
        }
        for path in own {
            assert!(
                from_home.contains(&inode(path)),
                "{} is not mounted: {mountinfo}",
                path.display()
            );
        }

        // Socket identity. The vsock socket directory is the one holding
        // the socket the VMM itself listens on, as the kernel names it.
        // The jail's only sockets are that listener and, beside it, its
        // host listeners for guest ports, `<listener>_<port>`
        // (Firecracker's vsock convention). No other host socket, the
        // project's session and shutdown control sockets and the other
        // box's vsock sockets among them, shares a device and inode with
        // anything in the jail.
        let bound = vmm.bound();
        let listening: Vec<&PathBuf> = bound
            .iter()
            .filter(|socket| socket.listening)
            .map(|socket| &socket.path)
            .collect();
        assert_eq!(
            listening.len(),
            1,
            "the VMM's listening sockets: {listening:?}"
        );
        let listener = listening[0];
        let dir = listener.parent().expect("a socket directory");
        let name = listener.file_name().unwrap().to_str().unwrap();
        let vsock_dir = inode(&vmm.root().join(dir.strip_prefix("/").unwrap()));
        for entry in seen.iter().filter(|entry| entry.socket) {
            let entry_name = entry.path.file_name().and_then(|n| n.to_str());
            let vsock_name = entry_name.is_some_and(|entry_name| {
                entry_name == name
                    || entry_name
                        .strip_prefix(name)
                        .and_then(|rest| rest.strip_prefix('_'))
                        .is_some_and(|port| {
                            !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
                        })
            });
            let parent = entry.path.parent().unwrap().strip_prefix("/").unwrap();
            assert!(
                vsock_name && inode(&vmm.root().join(parent)) == vsock_dir,
                "a socket in the jail that is not its vsock listener or a guest \
                 port's: {}",
                entry.path.display()
            );
        }
        let outside: Vec<&HostSocket> = host_sockets
            .iter()
            .filter(|socket| socket.dir != vsock_dir)
            .collect();
        assert!(
            outside.len() >= 3,
            "host sockets outside the jail: {host_sockets:?}"
        );
        for socket in outside {
            assert!(
                !inodes.contains(&socket.inode),
                "{} is in the jail",
                socket.path.display()
            );
        }

        // Every descriptor the VMM holds is in its jail, the spec's
        // `/dev/null` standard descriptors, a socket bound in its vsock
        // directory, or an anonymous kernel object.
        let null = inode(Path::new("/dev/null"));
        let in_vsock: BTreeSet<&str> = bound
            .iter()
            .filter(|socket| socket.path.parent() == Some(dir))
            .map(|socket| socket.inode.as_str())
            .collect();
        for fd in fs::read_dir(format!("/proc/{}/fd", vmm.pid)).unwrap() {
            let fd = fd.unwrap().path();
            let Ok(link) = fs::read_link(&fd) else {
                continue;
            };
            let text = link.to_string_lossy();
            if let Some(socket) = socket_inode(&link) {
                assert!(
                    in_vsock.contains(socket.as_str()),
                    "VMM {} holds {text}, not bound in its vsock directory: {bound:?}",
                    vmm.pid
                );
                continue;
            }
            if ["pipe:", "anon_inode:"]
                .iter()
                .any(|kind| text.starts_with(kind))
            {
                continue;
            }
            let held = inode(&fd);
            assert!(
                inodes.contains(&held) || held == null,
                "VMM {} holds {} ({text}) outside its jail",
                vmm.pid,
                fd.display()
            );
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

    /// The Unix sockets the VMM holds that are bound to a path, as its
    /// own network namespace's socket table names them.
    fn bound(&self) -> Vec<Bound> {
        let held: BTreeSet<String> = fs::read_dir(format!("/proc/{}/fd", self.pid))
            .unwrap()
            .filter_map(|entry| socket_inode(&fs::read_link(entry.ok()?.path()).ok()?))
            .collect();
        // `Num RefCount Protocol Flags Type St Inode Path`; Flags 00010000
        // is a listening socket.
        fs::read_to_string(format!("/proc/{}/net/unix", self.pid))
            .unwrap()
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let path = fields.get(7)?;
                held.contains(fields[6]).then(|| Bound {
                    path: PathBuf::from(path),
                    inode: fields[6].to_string(),
                    listening: fields[3] == "00010000",
                })
            })
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

#[derive(Debug)]
struct HostSocket {
    path: PathBuf,
    inode: (u64, u64),
    /// Its directory's device and inode.
    dir: (u64, u64),
}

/// Every socket file under `roots`, without following symlinks. Parallel
/// tests remove their directories under the shared runtime directory, so
/// an entry gone during the walk is skipped; any other error fails.
fn sockets_under(roots: &[PathBuf]) -> Vec<HostSocket> {
    fn gone<T>(result: std::io::Result<T>, path: &Path) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("{}: {error}", path.display()),
        }
    }
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dirs = roots.to_vec();
    while let Some(dir) = dirs.pop() {
        let Some(meta) = gone(fs::symlink_metadata(&dir), &dir) else {
            continue;
        };
        let dir_inode = (meta.dev(), meta.ino());
        if !meta.is_dir() || !seen.insert(dir_inode) {
            continue;
        }
        let Some(entries) = gone(fs::read_dir(&dir), &dir) else {
            continue;
        };
        for entry in entries {
            let Some(entry) = gone(entry, &dir) else {
                continue;
            };
            let path = entry.path();
            let Some(meta) = gone(fs::symlink_metadata(&path), &path) else {
                continue;
            };
            if meta.is_dir() {
                dirs.push(path);
            } else if meta.file_type().is_socket() {
                found.push(HostSocket {
                    inode: (meta.dev(), meta.ino()),
                    dir: dir_inode,
                    path,
                });
            }
        }
    }
    found
}

#[derive(Debug)]
struct Bound {
    path: PathBuf,
    inode: String,
    listening: bool,
}

/// The inode of a `socket:[N]` descriptor link.
fn socket_inode(link: &Path) -> Option<String> {
    link.to_str()?
        .strip_prefix("socket:[")?
        .strip_suffix(']')
        .map(str::to_string)
}

/// A mountinfo field with its octal escapes (`\040` and the like) decoded.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok());
        match (bytes[i], octal) {
            (b'\\', Some(byte)) => {
                out.push(byte);
                i += 4;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).expect("a UTF-8 mount point")
}
