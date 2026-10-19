// Prototype exec agent for a Firecracker box. Not pinfold code; std only.
//
// Guest: `pf-agent serve` runs under `socat VSOCK-LISTEN:1024,fork
// EXEC:...,nofork`, so stdin/stdout are the vsock connection.
// Host: `pf-agent exec UDS [--workdir D] -- ARGV...` connects to
// Firecracker's vsock Unix socket, sends `CONNECT 1024`, and relays.
// `pf-agent attach UDS` does the same for the guest's pty on port 1025.
// `pf-agent wait-ready PATH` accepts the guest's one readiness connection.
//
// Framing after the request: [kind u8][len u32 BE][payload].
// host->guest: 0 stdin bytes, 1 stdin EOF.
// guest->host: 1 stdout, 2 stderr, 3 exit (i32 BE; 128+signal).
// The connection closing before an exit frame cancels: the guest kills the
// command's whole process group.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

const BOX_UID: &str = "1000";

fn frame(w: &mut impl Write, kind: u8, data: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(5 + data.len());
    buf.push(kind);
    buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buf.extend_from_slice(data);
    w.write_all(&buf)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(head[1..5].try_into().unwrap()) as usize;
    let mut data = vec![0u8; len];
    r.read_exact(&mut data)?;
    Ok(Some((head[0], data)))
}

fn serve() -> io::Result<ExitCode> {
    let mut input = io::stdin();
    let output = Arc::new(Mutex::new(io::stdout()));
    let Some((_, req)) = read_frame(&mut input)? else {
        return Ok(ExitCode::FAILURE);
    };
    let mut fields = req.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());
    let workdir = fields.next().unwrap_or_default();
    let argv: Vec<String> = fields.collect();
    // Drop to the box user with an empty bounding set, as podman boxes run.
    let mut child = Command::new("setpriv")
        .args(["--reuid", BOX_UID, "--regid", BOX_UID, "--clear-groups"])
        .args(["--bounding-set", "-all", "--inh-caps", "-all", "--no-new-privs", "--"])
        .args(&argv)
        .current_dir(if workdir.is_empty() { "/workspace" } else { &workdir })
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", "/home/box")
        .env("HTTPS_PROXY", "http://127.0.0.1:3128")
        .env("http_proxy", "http://127.0.0.1:3128")
        .env("NODE_USE_ENV_PROXY", "1")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pgid = child.id();
    let pumps: Vec<_> = [(1u8, Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>),
                         (2u8, Box::new(child.stderr.take().unwrap()))]
        .into_iter()
        .map(|(kind, mut src)| {
            let out = Arc::clone(&output);
            thread::spawn(move || {
                let mut buf = [0u8; 32768];
                loop {
                    match src.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if frame(&mut *out.lock().unwrap(), kind, &buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
        })
        .collect();
    let mut stdin = child.stdin.take();
    thread::spawn(move || {
        loop {
            match read_frame(&mut input) {
                Ok(Some((0, data))) => {
                    if let Some(s) = stdin.as_mut() {
                        let _ = s.write_all(&data);
                    }
                }
                Ok(Some((1, _))) => stdin = None,
                Ok(Some(_)) => {}
                // The host went away: cancel the whole group.
                Ok(None) | Err(_) => {
                    let _ = Command::new("/bin/kill").args(["-KILL", "--", &format!("-{pgid}")]).status();
                    std::process::exit(137);
                }
            }
        }
    });
    let status = child.wait()?;
    for p in pumps {
        let _ = p.join();
    }
    let code = status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    frame(&mut *output.lock().unwrap(), 3, &code.to_be_bytes())?;
    Ok(ExitCode::SUCCESS)
}

fn connect(uds: &str, port: u32) -> io::Result<UnixStream> {
    let mut s = UnixStream::connect(uds)?;
    s.write_all(format!("CONNECT {port}\n").as_bytes())?;
    let mut line = String::new();
    // Read the reply byte by byte so no relayed bytes are buffered away.
    let mut b = [0u8; 1];
    while s.read(&mut b)? == 1 {
        line.push(b[0] as char);
        if b[0] == b'\n' {
            break;
        }
    }
    if !line.starts_with("OK ") {
        return Err(io::Error::other(format!("vsock connect refused: {line:?}")));
    }
    Ok(s)
}

fn exec(uds: &str, workdir: &str, argv: &[String]) -> io::Result<ExitCode> {
    let mut s = connect(uds, 1024)?;
    let mut req = workdir.as_bytes().to_vec();
    for a in argv {
        req.push(0);
        req.extend_from_slice(a.as_bytes());
    }
    frame(&mut s, 0, &req)?;
    let mut w = s.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 32768];
        let mut stdin = io::stdin().lock();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = frame(&mut w, 1, &[]);
                    break;
                }
                Ok(n) => {
                    if frame(&mut w, 0, &buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let (mut out, mut err) = (io::stdout().lock(), io::stderr().lock());
    while let Some((kind, data)) = read_frame(&mut s)? {
        match kind {
            1 => { out.write_all(&data)?; out.flush()?; }
            2 => { err.write_all(&data)?; err.flush()?; }
            3 => return Ok(ExitCode::from(i32::from_be_bytes(data[..4].try_into().unwrap()) as u8)),
            _ => {}
        }
    }
    Err(io::Error::other("box closed the exec without an exit code"))
}

fn attach(uds: &str) -> io::Result<ExitCode> {
    let s = connect(uds, 1025)?;
    let mut w = s.try_clone()?;
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut w);
        let _ = w.shutdown(std::net::Shutdown::Write);
    });
    io::copy(&mut BufReader::new(s), &mut io::stdout().lock())?;
    Ok(ExitCode::SUCCESS)
}

fn wait_ready(path: &str) -> io::Result<ExitCode> {
    let l = UnixListener::bind(path)?;
    let (s, _) = l.accept()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    print!("{line}");
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("serve") => serve(),
        Some("exec") if args.len() > 3 => {
            let (mut workdir, mut i) = (String::new(), 3);
            if args[3] == "--workdir" {
                workdir = args[4].clone();
                i = 5;
            }
            if args.get(i).map(String::as_str) == Some("--") {
                i += 1;
            }
            exec(&args[2], &workdir, &args[i..])
        }
        Some("attach") if args.len() == 3 => attach(&args[2]),
        Some("wait-ready") if args.len() == 3 => wait_ready(&args[2]),
        _ => Err(io::Error::other("usage: pf-agent serve | exec UDS [--workdir D] -- ARGV... | attach UDS | wait-ready PATH")),
    };
    result.unwrap_or_else(|e| {
        eprintln!("pf-agent: {e}");
        ExitCode::from(125)
    })
}
