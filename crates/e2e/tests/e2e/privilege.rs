//! Guarantee 5: box work runs as the host uid with no capabilities, cannot
//! regain privileges, and each box boots its own kernel.

use std::collections::BTreeMap;
use std::process::Command;

use e2e::{TestEnv, box_name, default_image, run};
use serde_json::json;

// Linux errno values, from the guest kernel on both runtimes.
const EACCES: &str = "13";
const ENOSPC: &str = "28";
const EROFS: &str = "30";
const NO_CAPABILITIES: &str = "0000000000000000";
/// unshare(2), from the kernel's syscall table for the guest's architecture.
#[cfg(target_arch = "aarch64")]
const SYS_UNSHARE: u32 = 97;
#[cfg(target_arch = "x86_64")]
const SYS_UNSHARE: u32 = 272;
const CLONE_NEWUSER: u32 = 0x1000_0000;

/// `/proc/PID/status`'s identity fields, as `perl` prints them in the box.
fn status(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_string(), value.trim().to_string()))
        .collect()
}

/// A perl snippet that prints `/proc/<$pid>/status`'s identity lines.
const PRINT_STATUS: &str = "open(my $f, '<', \"/proc/$pid/status\") or die \"status: $!\\n\"; \
     print grep { /^(Uid|Gid|PPid|Cap(Inh|Prm|Eff|Bnd|Amb)|NoNewPrivs):/ } <$f>;";

/// Walk the tree under `$ARGV[0]` without following links, skipping the
/// mounts that are not the image: the kernel's `/proc`, `/sys` and `/dev`,
/// and the tmpfs `/tmp` (Always applied). Device numbers are not compared:
/// an overlay root reports several. Print `setid PATH` for each setuid or
/// setgid file and `closed DIR ERRNO` for each directory it cannot list,
/// with chdir's result there (`0` if it can still be searched), then `done`.
const SETID_SCAN: &str = "my %skip = map { $_ => 1 } qw(/proc /sys /dev /tmp); \
     sub walk { my $dir = shift; \
       opendir(my $h, $dir) or do { print \"closed $dir \", (chdir($dir) ? 0 : $! + 0), \"\\n\"; return }; \
       for my $entry (sort readdir $h) { \
         next if $entry eq '.' || $entry eq '..'; \
         my $path = $dir eq '/' ? \"/$entry\" : \"$dir/$entry\"; \
         my @stat = lstat $path or next; \
         next if $skip{$path}; \
         if (-d _) { walk($path) } elsif (-f _ && ($stat[2] & 06000)) { print \"setid $path\\n\" } \
       } } \
     walk($ARGV[0]); print \"done\\n\"";

fn host_uid() -> String {
    let out = run(
        Command::new("id").arg("-u"),
        std::time::Duration::from_secs(10),
    );
    assert_eq!(out.code, 0, "id -u: {out:?}");
    out.stdout.trim().to_string()
}

#[test]
fn nothing_can_gain_privileges() {
    // Sabotage (run): on Apple, drop `--cap-drop ALL` from `container run`
    // in runtime/apple.rs. The grandchild's CapBnd is then the runtime's
    // default set, not 0. On Linux, drop `drop_bounding(0)` from
    // `become_box_user` in init/firecracker.rs: CapBnd is then PID 1's full
    // set. Each block below names its own.
    // The plain child and unshare(0) are the positive controls.
    let image = default_image();
    let env = TestEnv::new("privilege");
    let uid = host_uid();
    let name = box_name("privilege");
    let _up = env.up(&json!({ "name": name, "image": image }));

    // The exec'd perl forks a child, which forks the grandchild, which execs
    // a fresh perl. That one reports its identity, runs a plain child, and
    // tries unshare with and without CLONE_NEWUSER.
    let grandchild = format!(
        "my $pid = 'self'; {PRINT_STATUS} \
         print 'plain: ', system('true'), \"\\n\"; \
         print 'unshare 0: ', (syscall({SYS_UNSHARE}, 0) == -1 ? $! + 0 : 0), \"\\n\"; \
         print 'unshare user: ', (syscall({SYS_UNSHARE}, {CLONE_NEWUSER}) == -1 ? $! + 0 : 0), \"\\n\";"
    );
    let parent = "defined(my $child = fork) or die \"fork: $!\\n\"; \
         if ($child == 0) { \
           defined(my $grandchild = fork) or die \"fork: $!\\n\"; \
           exec('perl', '-e', $ARGV[0]) or die \"exec: $!\\n\" if $grandchild == 0; \
           waitpid($grandchild, 0); exit($? >> 8); \
         } \
         waitpid($child, 0); exit($? >> 8);";
    let out = env.exec(&name, &["perl", "-e", parent, &grandchild]);
    assert_eq!(out.code, 0, "the grandchild probe: {out:?}");
    let seen = status(&out.stdout);
    let ids = format!("{uid}\t{uid}\t{uid}\t{uid}");
    assert_eq!(seen["Uid"], ids, "{}", out.stdout);
    for set in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
        assert_eq!(seen[set], NO_CAPABILITIES, "{set}: {}", out.stdout);
    }
    assert_eq!(seen["NoNewPrivs"], "1", "{}", out.stdout);
    // Control: a plain child runs, and unshare itself works without a new
    // user namespace; with one it fails on the namespace limit of 0.
    // Sabotage: drop the `user/max_user_namespaces` write from `prepare` in
    // init/firecracker.rs. `unshare user` then returns 0.
    assert_eq!(seen["plain"], "0", "{}", out.stdout);
    assert_eq!(seen["unshare 0"], "0", "{}", out.stdout);
    assert_eq!(seen["unshare user"], ENOSPC, "{}", out.stdout);

    // No image-provided setuid or setgid file that box work can reach:
    // each directory it cannot list it cannot search either (EACCES),
    // so nothing below one can be executed.
    // Sabotage: drop the `find / -xdev -perm /6000` line from the bundled
    // profile/default/Containerfile. The scan then prints `setid` lines.
    // On the Mac, delete the reused profile-default image first.
    let scan = |root: &str| {
        let out = env.exec(&name, &["perl", "-e", SETID_SCAN, root]);
        assert_eq!(out.code, 0, "the scan of {root}: {out:?}");
        out.stdout
    };
    let root = scan("/");
    assert_eq!(root.lines().last(), Some("done"), "{root}");
    for line in root.lines().filter(|line| *line != "done") {
        assert!(
            line.starts_with("closed ") && line.ends_with(&format!(" {EACCES}")),
            "{root}"
        );
    }
    // Control: the same scan finds a setuid file the box makes in `/tmp`.
    let planted = env.exec(
        &name,
        &["sh", "-c", "touch /tmp/setid && chmod 4755 /tmp/setid"],
    );
    assert_eq!(planted.code, 0, "{planted:?}");
    assert_eq!(scan("/tmp"), "setid /tmp/setid\ndone\n");

    // The root refuses a write as read-only; the tmpfs `/tmp` takes it.
    // Sabotage: on Apple, drop `--read-only` from `container run`; on Linux,
    // drop `MS_RDONLY` from the root's remount in init/firecracker.rs. The
    // first write then fails with EACCES, not EROFS.
    let write = |path: &str| {
        let out = env.exec(
            &name,
            &[
                "perl",
                "-e",
                "print open(my $f, '>', $ARGV[0]) ? 0 : $! + 0",
                path,
            ],
        );
        assert_eq!(out.code, 0, "{out:?}");
        out.stdout
    };
    assert_eq!(write("/pinfold-e2e-probe"), EROFS);
    assert_eq!(write("/tmp/pinfold-e2e-probe"), "0");

    // On Linux PID 1 keeps only CAP_SYS_ADMIN (bit 21), and the exec
    // service, PID 1's child that started this command, holds nothing.
    // Sabotage: drop the keep-only-CAP_SYS_ADMIN step from `boot` in
    // init/firecracker.rs, and PID 1 keeps every set. For the service, set
    // `PR_SET_KEEPCAPS` before `setresuid` in `become_box_user` and drop
    // `set_capabilities(None)`: the service keeps CapPrm, which exec clears
    // for the grandchild above, so only this block sees it.
    #[cfg(target_os = "linux")]
    {
        let probe = |pid: &str| {
            let out = env.exec(
                &name,
                &["perl", "-e", &format!("my $pid = {pid}; {PRINT_STATUS}")],
            );
            assert_eq!(out.code, 0, "{out:?}");
            status(&out.stdout)
        };
        let init = probe("1");
        for set in ["CapPrm", "CapEff", "CapBnd"] {
            assert_eq!(init[set], "0000000000200000", "PID 1's {set}: {init:?}");
        }
        for set in ["CapInh", "CapAmb"] {
            assert_eq!(init[set], NO_CAPABILITIES, "PID 1's {set}: {init:?}");
        }
        let service = probe("getppid()");
        assert_eq!(service["PPid"], "1", "the exec service: {service:?}");
        assert_eq!(service["Uid"], ids, "the exec service: {service:?}");
        for set in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
            assert_eq!(
                service[set], NO_CAPABILITIES,
                "the exec service: {service:?}"
            );
        }
    }

    // A second box boots another kernel: a different boot_id, and on Linux
    // neither is the host's.
    // Sabotage: in `exec` in runtime/firecracker/mod.rs, open the first
    // box's sockets whatever the name. Every check above still passes;
    // the two boot_ids then match.
    let other = box_name("privilege-other");
    let _other = env.up(&json!({ "name": other, "image": image }));
    let boot_id = |name: &str| {
        let out = env.exec(name, &["cat", "/proc/sys/kernel/random/boot_id"]);
        assert_eq!(out.code, 0, "{out:?}");
        out.stdout.trim().to_string()
    };
    let (first, second) = (boot_id(&name), boot_id(&other));
    assert_ne!(first, second, "two boxes share a kernel");
    #[cfg(target_os = "linux")]
    {
        let host = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        assert!(
            ![&first, &second].contains(&&host.trim().to_string()),
            "a box runs on the host's kernel"
        );
    }
}
