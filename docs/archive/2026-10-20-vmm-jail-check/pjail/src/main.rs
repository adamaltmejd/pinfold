// Scratch driver for pinfold's VMM jail module (not pinfold code): it plays
// `box up`'s part. `pjail probe` runs preflight's user-namespace probe.
//   pjail STATE FIRECRACKER KERNEL ROOTFS INITDISK DISK...
// STATE holds config.json, vsock/ and an empty jail/.
#[path = "../../pinfold/crates/pinfold/src/core/runtime/firecracker/jail.rs"]
mod jail;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("probe") {
        match jail::probe() {
            Ok(()) => println!("probe ok"),
            Err(error) => {
                println!("probe failed: {error}");
                std::process::exit(1)
            }
        }
        return;
    }
    let state = PathBuf::from(&args[0]);
    let (fc, kernel, rootfs, init) = (
        PathBuf::from(&args[1]),
        PathBuf::from(&args[2]),
        PathBuf::from(&args[3]),
        PathBuf::from(&args[4]),
    );
    let disks: Vec<PathBuf> = args[5..].iter().map(PathBuf::from).collect();
    let root = state.join("jail");
    let vsock = state.join("vsock");
    let config = state.join("config.json");
    let vmm = jail::spawn(&jail::Jail {
        root: &root,
        firecracker: &fc,
        kernel: &kernel,
        root_disk: &rootfs,
        init_disk: &init,
        disks: &disks,
        vsock: &vsock,
        config: &config,
    })
    .unwrap_or_else(|error| {
        eprintln!("pjail: {error}");
        std::process::exit(125)
    });
    std::fs::write(state.join("vmm.pid"), format!("{}\n", vmm.pid)).unwrap();
    let code = vmm.wait().unwrap();
    eprintln!("pjail: vmm {} ended, status {code}", vmm.pid);
    std::process::exit(code)
}
