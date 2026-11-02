//! Send a disk tree as bytes, never executing anything from that disk.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::{STREAM_DIR, STREAM_END, STREAM_EXECUTABLE, STREAM_FILE, STREAM_SYMLINK};

pub(crate) const ENTRIES: u64 = 65_536;
pub(crate) const PATH_BYTES: usize = 4096;
pub(crate) const METADATA_BYTES: u64 = 16 << 20;
pub(crate) const CONTENT_BYTES: u64 = 64 << 30;
pub(crate) const ABORT: u8 = b'!';

pub(crate) fn run(args: &[String]) -> io::Result<()> {
    let [root] = args else {
        return Err(io::Error::other("usage: pinfold init send DIR"));
    };
    let root = Path::new(root);
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("spec: export root is not a directory"));
    }
    let mut budget = Budget::default();
    let mut output = io::BufWriter::new(io::stdout().lock());
    if let Err(error) = send(root, Path::new(""), &mut output, &mut budget) {
        let reason = match error.kind() {
            io::ErrorKind::FileTooLarge => b'l',
            io::ErrorKind::InvalidData => b's',
            _ => b'f',
        };
        let _ = output.write_all(&[ABORT, reason]);
        let _ = output.flush();
        return Err(error);
    }
    output.write_all(&[STREAM_END])?;
    output.flush()
}

#[derive(Default)]
struct Budget {
    entries: u64,
    metadata: u64,
    content: u64,
}

fn send(
    root: &Path,
    relative: &Path,
    output: &mut impl Write,
    budget: &mut Budget,
) -> io::Result<()> {
    // Depth and byte limits also bound stack usage and temporary paths.
    if relative.components().count() > 256 {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "export nesting",
        ));
    }
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let bytes = path.as_os_str().as_bytes();
        budget.entries += 1;
        budget.metadata += bytes.len() as u64;
        if budget.entries > ENTRIES || bytes.len() > PATH_BYTES || budget.metadata > METADATA_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "export metadata",
            ));
        }
        let source = root.join(&path);
        let metadata = fs::symlink_metadata(&source)?;
        let kind = metadata.file_type();
        let tag = if kind.is_dir() {
            STREAM_DIR
        } else if kind.is_symlink() {
            STREAM_SYMLINK
        } else if kind.is_file() {
            if metadata.permissions().mode() & 0o111 == 0 {
                STREAM_FILE
            } else {
                STREAM_EXECUTABLE
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "export contains a special file",
            ));
        };
        output.write_all(&[tag])?;
        output.write_all(&(bytes.len() as u32).to_be_bytes())?;
        output.write_all(bytes)?;
        match tag {
            STREAM_DIR => {
                // mkfs leaves this root-owned recovery directory inaccessible
                // to the ordinary box user. Preserve the directory, not its
                // inaccessible contents; a user-owned namesake is ordinary data.
                let housekeeping = relative.as_os_str().is_empty()
                    && entry.file_name() == "lost+found"
                    && metadata.uid() == 0
                    && fs::read_dir(&source)
                        .is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied);
                if !housekeeping {
                    send(root, &path, output, budget)?;
                }
            }
            STREAM_SYMLINK => {
                let target = fs::read_link(source)?;
                let target = target.as_os_str().as_bytes();
                budget.metadata += target.len() as u64;
                output.write_all(&(target.len() as u32).to_be_bytes())?;
                if target.len() > PATH_BYTES || budget.metadata > METADATA_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        "export symlink",
                    ));
                }
                output.write_all(target)?;
            }
            _ => {
                let mut file = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
                    .open(source)?;
                let metadata = file.metadata()?;
                if !metadata.is_file() {
                    return Err(io::Error::other("spec: export source changed type"));
                }
                let size = metadata.len();
                output.write_all(&size.to_be_bytes())?;
                budget.content = budget
                    .content
                    .checked_add(size)
                    .filter(|sum| *sum <= CONTENT_BYTES)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::FileTooLarge, "export content"))?;
                if io::copy(&mut Read::by_ref(&mut file).take(size), output)? != size {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
            }
        }
    }
    Ok(())
}
