use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use pinfold::{cli, init};

/// One line per verb from the CLI table in docs/ARCHITECTURE.md, plus the
/// options that answer before a verb is chosen. Printed on stdout by
/// `--help` and on stderr for a malformed invocation.
const USAGE: &str = "\
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [--box NAME] [cmd…]   bash (or cmd) in this project's running pi box
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim disk (see Maintenance)
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold config [ROOT]            the effective configuration and project facts as JSON, for callers
pinfold box …                    the process interface
pinfold init                     PID 1 in the box (Linux builds)
pinfold --version                print the version
pinfold --help                   print this usage";

fn main() -> ExitCode {
    // The one binary doubles as PID 1 in a box on Linux and as the `pi` shim.
    let mut args = std::env::args_os();
    let program = args.next();
    let rest: Vec<OsString> = args.collect();
    // A symlink named `pi` makes argv[0] the shim; every argument after it is
    // pi's.
    let shim = program
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("pi");
    let verb = rest.first().and_then(|verb| verb.to_str());
    // The daily pass runs before any command, the `pi` shim included. Only a
    // verb that does work pays for it: `init` is PID 1 in a box with the
    // project home for state and no runtime to prune, and `--version`,
    // `--help`, a bare `pinfold` and an unknown verb touch nothing.
    let working = matches!(
        verb,
        Some(
            "box" | "build" | "pi" | "clean" | "doctor" | "config" | "profile" | "allow" | "attach"
        )
    );
    if shim || working {
        pinfold::core::clean::maintain();
    }
    if shim {
        return ExitCode::from(cli::pi(&rest) as u8);
    }
    match verb {
        Some("init") => init::run(&rest[1..]),
        Some("--version" | "-V") => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("box") => ExitCode::from(cli::run(&rest[1..]) as u8),
        Some("allow") => ExitCode::from(cli::allow(&rest[1..]) as u8),
        Some("attach") => ExitCode::from(cli::attach(&rest[1..]) as u8),
        Some("build") => ExitCode::from(cli::build(&rest[1..]) as u8),
        Some("clean") => ExitCode::from(cli::clean(&rest[1..]) as u8),
        Some("doctor") => ExitCode::from(cli::doctor(&rest[1..]) as u8),
        Some("config") => ExitCode::from(cli::config(&rest[1..]) as u8),
        Some("profile") => ExitCode::from(cli::profile(&rest[1..]) as u8),
        Some("pi") => ExitCode::from(cli::pi(&rest[1..]) as u8),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(1)
        }
    }
}
