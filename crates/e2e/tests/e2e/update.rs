//! Host self-update through a real HTTPS release fixture.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use e2e::{TestEnv, default_image, pinfold, run_ok};

use crate::box_::{ChildOwner, box_name, box_up, read_bounded};

#[test]
fn host_updates_are_verified_and_atomic() {
    let _runtime = crate::shared_runtime();
    // Guarantee 28. Sabotage: skip SHA256SUMS verification or its digest
    // comparison; the wrong digest under the requested asset name installs.
    // Compare a digest without selecting the asset name; the correct digest
    // under another name authorizes the release. Rename before
    // verification; the old inode changes on refusal. Write into the
    // executable instead of renaming; the final inode stays unchanged.
    // Replace argv[0] rather than current_exe(); the invoked
    // command symlink is replaced and the installed binary's inode stays
    // unchanged. Remove the exclusive executable lock; the owner under a
    // different XDG root no longer prevents updating its executable.
    // Expected release bytes, digest and version come from the host fixture;
    // inode identity comes from the host filesystem.
    let env = TestEnv::new("update");
    let installed = env.root.join("pinfold");
    fs::copy(pinfold(), &installed).unwrap();
    let update_alias = env.root.join("pinfold-link");
    symlink("pinfold", &update_alias).unwrap();
    let mut fixture = ReleaseProxy::new(&env);
    let original = fs::read(&installed).unwrap();
    let original_inode = fs::metadata(&installed).unwrap().ino();
    let replacement = changed_binary(&env);
    let replacement_bytes = fs::read(&replacement).unwrap();
    assert_ne!(
        replacement_bytes, original,
        "the release fixture must differ from the installed bytes"
    );
    fs::copy(replacement, fixture.root.join("release")).unwrap();

    let bad_checksum = fixture.root.join("bad-checksum");
    fs::write(&bad_checksum, b"").unwrap();
    let failure = fixture
        .command(&env, &installed)
        .arg("update")
        .output()
        .unwrap();
    assert!(
        !failure.status.success(),
        "wrong-name checksum authorized the release: {failure:?}"
    );
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("checksum-mismatch"),
        "wrong refusal: {failure:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    fs::remove_file(bad_checksum).unwrap();

    let name = box_name("update");
    let spec = serde_json::json!({ "name": name, "image": default_image(&env) });
    let other = TestEnv::new("update-owner");
    // Sabotage: hold the exclusive executable lock only before downloading.
    // A working command starts after the fixture observes the asset
    // request; replacement must recheck it before its atomic rename. The
    // command is a `box exec` of the installed executable under another XDG
    // root: exec needs no jail, so AppArmor's profile does not apply, and
    // it holds the executable lock until its command ends.
    let mut boxed = box_up(&other, &spec, &name);
    fs::write(fixture.root.join("hold-download"), b"").unwrap();
    let mut downloading = fixture.command(&env, &installed);
    let update = downloading
        .arg("update")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fixture.wait_for_download();
    let child = other
        .command(&installed)
        .args([
            "box",
            "exec",
            &name,
            "--",
            "sh",
            "-c",
            "echo ready; exec cat",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut owner = ChildOwner::new(child, &other, None);
    let mut stdout = BufReader::new(owner.stdout.take().unwrap());
    let line = read_bounded(&mut owner, &mut stdout, Duration::from_secs(60), false)
        .expect("read the exec owner's readiness");
    assert_eq!(line.trim(), "ready", "the exec owner did not start");
    fixture.release_download();
    let refusal = update.wait_with_output().unwrap();
    assert!(
        !refusal.status.success(),
        "a command started during the download was replaced"
    );
    assert!(
        String::from_utf8_lossy(&refusal.stderr).contains("update-busy"),
        "wrong race refusal: {refusal:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), original);
    assert_eq!(fs::metadata(&installed).unwrap().ino(), original_inode);
    drop(owner.stdin.take());
    owner.wait_bounded();
    boxed.down(&other);
    boxed.wait();
    fs::remove_file(fixture.root.join("hold-download")).unwrap();

    run_ok(fixture.command(&env, &update_alias).arg("update"));
    assert_eq!(fs::read(&installed).unwrap(), replacement_bytes);
    assert_ne!(fs::metadata(&installed).unwrap().ino(), original_inode);
    assert_eq!(fs::read_link(&update_alias).unwrap(), Path::new("pinfold"));
    run_ok(env.command(&installed).arg("--version"));
}

fn changed_binary(env: &TestEnv) -> PathBuf {
    // A different valid executable: the suite's binary with bytes appended
    // past its end, which the loader never maps.
    let mut bytes = fs::read(pinfold()).unwrap();
    bytes.extend_from_slice(b"pinfold update fixture\n");
    let installed = env.root.join("changed-pinfold");
    fs::write(&installed, bytes).unwrap();
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).unwrap();
    installed
}

struct ReleaseProxy {
    child: Child,
    events: BufReader<ChildStdout>,
    root: PathBuf,
    port: u16,
}

impl ReleaseProxy {
    fn new(env: &TestEnv) -> Self {
        let root = env.root.join("release-fixture");
        fs::create_dir(&root).unwrap();
        fs::copy(pinfold(), root.join("release")).unwrap();
        run_ok(Command::new("openssl").current_dir(&root).args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=api.github.com",
            "-addext",
            "subjectAltName=DNS:api.github.com,DNS:github.com",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
        ]));
        let target = if cfg!(target_os = "macos") {
            "aarch64-apple-darwin".to_string()
        } else {
            format!("{}-unknown-linux-musl", std::env::consts::ARCH)
        };
        let mut child = Command::new("python3")
            .args(["-c", include_str!("release_proxy.py")])
            .arg(&root)
            .arg(format!("pinfold-999.0.0-{target}"))
            .stdout(Stdio::piped())
            .spawn()
            .expect("start host HTTPS proxy");
        let mut line = String::new();
        let mut events = BufReader::new(child.stdout.take().unwrap());
        events.read_line(&mut line).expect("read fixture readiness");
        let port = line.trim().parse().expect("fixture reports its port");
        Self {
            child,
            events,
            root,
            port,
        }
    }

    fn wait_for_download(&mut self) {
        let mut line = String::new();
        self.events.read_line(&mut line).unwrap();
        assert_eq!(
            line.trim(),
            "download",
            "release fixture did not observe an asset request"
        );
    }

    fn release_download(&self) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.write_all(b"RELEASE\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "released\n");
    }

    fn command(&self, env: &TestEnv, binary: &Path) -> Command {
        let mut command = env.command(binary);
        command
            .env("HTTPS_PROXY", format!("http://127.0.0.1:{}", self.port))
            .env("CURL_CA_BUNDLE", self.root.join("cert.pem"))
            .env_remove("https_proxy")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy");
        command
    }
}

impl Drop for ReleaseProxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
