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

        // No directory under the home is mounted in the jail: what comes
        // from the home (the state root, so the disks) is single files.
        // mountinfo's fourth field is the mount's root in its filesystem,
        // the fifth its point in the jail.
        let mountinfo = fs::read_to_string(format!("/proc/{}/mountinfo", vmm.pid)).unwrap();
        let mut from_home = 0;
        for line in mountinfo.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            if Path::new(fields[3]).starts_with(&home) {
                from_home += 1;
                let point = vmm.root().join(fields[4].trim_start_matches('/'));
                assert!(
                    !fs::symlink_metadata(&point).unwrap().is_dir(),
                    "a home directory in the jail: {line}"
                );
            }
        }
        assert!(from_home >= own.len(), "{mountinfo}");

        // Socket identity. The vsock socket directory is the one holding
        // the socket the VMM itself listens on, as the kernel names it.
        // Only sockets in that directory are in the jail; no other host
        // socket, the project's session and shutdown control sockets and
        // the other box's vsock sockets among them, shares a device and
        // inode with anything in the jail.
        let listening = vmm.listening();
        assert_eq!(
            listening.len(),
            1,
            "the VMM's listening sockets: {listening:?}"
        );
        let vsock_dir = inode(
            &vmm.root().join(
                listening[0]
                    .parent()
                    .expect("a socket directory")
                    .strip_prefix("/")
                    .expect("an absolute socket path"),
            ),
        );
        for entry in seen.iter().filter(|entry| entry.socket) {
            let dir = entry.path.parent().unwrap().strip_prefix("/").unwrap();
            assert_eq!(
                inode(&vmm.root().join(dir)),
                vsock_dir,
                "a socket in the jail outside its vsock directory: {}",
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
        // `/dev/null` standard descriptors, or an anonymous kernel object.
        let null = inode(Path::new("/dev/null"));
        for fd in fs::read_dir(format!("/proc/{}/fd", vmm.pid)).unwrap() {
            let fd = fd.unwrap().path();
            let Ok(link) = fs::read_link(&fd) else {
                continue;
            };
            let text = link.to_string_lossy();
            if ["socket:", "pipe:", "anon_inode:"]
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

    /// The paths, as the VMM sees them, of the Unix sockets it listens
    /// on: its descriptors' socket inodes, named in its own network
    /// namespace's socket table.
    fn listening(&self) -> Vec<PathBuf> {
        let held: BTreeSet<String> = fs::read_dir(format!("/proc/{}/fd", self.pid))
            .unwrap()
            .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
            .filter_map(|link| {
                link.to_str()?
                    .strip_prefix("socket:[")?
                    .strip_suffix(']')
                    .map(str::to_string)
            })
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
                (fields[3] == "00010000" && held.contains(fields[6])).then(|| PathBuf::from(path))
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

/// Every socket file under `roots`, without following symlinks.
fn sockets_under(roots: &[PathBuf]) -> Vec<HostSocket> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dirs = roots.to_vec();
    while let Some(dir) = dirs.pop() {
        let Ok(meta) = fs::symlink_metadata(&dir) else {
            continue;
        };
        if !meta.is_dir() || !seen.insert((meta.dev(), meta.ino())) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                dirs.push(path);
            } else if meta.file_type().is_socket() {
                found.push(HostSocket {
                    inode: (meta.dev(), meta.ino()),
                    dir: inode(&dir),
                    path,
                });
            }
        }
    }
    found
}
