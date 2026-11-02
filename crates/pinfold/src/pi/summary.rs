//! Advisory exit reports. No guest data crosses into a host checkout.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::core::r#box::{Box, Shutdown, Signals};
use crate::core::plan::Plan;
use crate::core::runs::Terminal;
use crate::core::runtime::runtime;
use crate::init::summary::{BYTES, ENTRIES, Entry, Summary};

pub async fn report(
    box_: &mut Box,
    signals: &mut Signals,
    plan: &Plan,
    init: &Path,
    root: &Path,
    repository: bool,
) -> io::Result<Option<(Terminal, i32)>> {
    let mode = if repository { "repository" } else { "scratch" };
    let result = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(2), collect(&plan.name, init, root, mode)) => result.ok().and_then(Result::ok),
        signal = signals.recv() => return Ok(Some((Shutdown::Signal.terminal(), 128 + signal as i32))),
        ended = box_.ended() => {
            let shutdown = ended?;
            let code = match shutdown {
                Shutdown::BoxExited(status) => crate::cli::exit_code(status),
                Shutdown::AuditLog => 1,
                Shutdown::StdinEof | Shutdown::Signal => 0,
            };
            return Ok(Some((shutdown.terminal(), code)));
        },
    };
    let Some(summary) = result else {
        eprintln!("pinfold pi: summary-unavailable; disks retained");
        return Ok(None);
    };
    if summary.entries.is_empty() && !summary.truncated {
        return Ok(None);
    }
    let disks = &plan.disks;
    let retained = serde_json::json!({
        "home": disks.first().map(|disk| &disk.name),
        "work": disks.get(1).map(|disk| &disk.name),
    });
    let entries: Vec<_> = summary
        .entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "kind": entry.kind, "path": entry.path, "branch": entry.branch,
                "bytes": entry.bytes, "commits": entry.commits,
                "added": entry.added, "deleted": entry.deleted,
            })
        })
        .collect();
    let mut value = serde_json::json!({"entries": entries, "truncated": summary.truncated});
    value["mode"] = mode.into();
    value["disks"] = retained;
    let mut encoded = value.to_string();
    while encoded.len() > BYTES {
        value["entries"]
            .as_array_mut()
            .expect("entries is an array")
            .pop();
        value["truncated"] = true.into();
        encoded = value.to_string();
    }
    eprintln!("pinfold pi: summary {encoded}");
    if repository {
        eprintln!(
            "pinfold pi: use `pinfold fetch` for committed branches; uncommitted files stay on the disk"
        );
    } else if let Some(work) = disks.get(1) {
        eprintln!(
            "pinfold pi: guest files are untrusted; export with `pinfold disk export {} NEW_DIRECTORY`",
            work.name
        );
    }
    Ok(None)
}

async fn collect(name: &str, init: &Path, root: &Path, mode: &str) -> io::Result<Summary> {
    let baseline = if mode == "repository" {
        host_branches(root).await?
    } else {
        Vec::new()
    };
    let root = super::launch::utf8(root, "summary root")?;
    let argv = [
        "/proc/self/exe".into(),
        "init".into(),
        "summary".into(),
        mode.into(),
        root.into(),
    ];
    let mut command = runtime().exec_command(name, init, false, None, &argv);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("summary stdin"))?
        .write_all(&baseline)
        .await?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("summary pipe"))?
        .take((BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > BYTES || !child.wait().await?.success() {
        return Err(io::Error::other("summary failed"));
    }
    if !bytes.ends_with(b"\n")
        || bytes
            .iter()
            .any(|byte| !matches!(*byte, b'\n' | b'\t' | 0x20..=0x7e))
    {
        return Err(io::Error::other("summary invalid"));
    }
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let mut lines = text.lines();
    let truncated = match lines.next() {
        Some("pinfold-summary-v1\t0") => false,
        Some("pinfold-summary-v1\t1") => true,
        _ => return Err(io::Error::other("summary invalid")),
    };
    let mut summary = Summary {
        entries: Vec::new(),
        truncated,
    };
    for line in lines {
        let fields: Vec<_> = line.split('\t').collect();
        let [kind, path, bytes, commits, branch, added, deleted] = fields.as_slice() else {
            return Err(io::Error::other("summary invalid"));
        };
        if summary.entries.len() == ENTRIES
            || path.len() > 4096
            || branch.len() > 4096
            || !matches!(
                *kind,
                "file"
                    | "directory"
                    | "symlink"
                    | "special"
                    | "committed"
                    | "tracked"
                    | "untracked"
            )
            || [added, deleted].into_iter().any(|count| {
                !count.is_empty()
                    && *count != "-"
                    && !count.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(io::Error::other("summary invalid"));
        }
        let number = |value: &str| -> io::Result<Option<u64>> {
            if value == "-" {
                Ok(None)
            } else {
                value.parse().map(Some).map_err(io::Error::other)
            }
        };
        summary.entries.push(Entry {
            kind: kind.to_string(),
            path: path.to_string(),
            branch: (!branch.is_empty()).then(|| branch.to_string()),
            bytes: number(bytes)?,
            commits: number(commits)?,
            added: (!added.is_empty()).then(|| added.to_string()),
            deleted: (!deleted.is_empty()).then(|| deleted.to_string()),
        });
    }
    Ok(summary)
}

async fn host_branches(root: &Path) -> io::Result<Vec<u8>> {
    let mut command = crate::core::crossing::git_command(root);
    command
        .args(["for-each-ref", "--format=%(objectname)", "refs/heads/"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("summary baseline"))?
        .take((4096 * 65 + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 4096 * 65 || !child.wait().await?.success() {
        return Err(io::Error::other("summary baseline limit"));
    }
    Ok(bytes)
}
