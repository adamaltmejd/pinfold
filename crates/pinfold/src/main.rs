use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use pinfold::{cli, init};

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
    // The daily pass runs before any command, the `pi` shim included. Only
    // `pinfold init` skips it: PID 1 in a box has the project home for state
    // and no runtime to prune.
    if shim || verb != Some("init") {
        pinfold::core::clean::maintain();
    }
    if shim {
        return ExitCode::from(cli::pi(&rest) as u8);
    }
    match verb {
        Some("init") => init::run(&rest[1..]),
        Some("box") => ExitCode::from(cli::run(&rest[1..]) as u8),
        Some("allow") => ExitCode::from(cli::allow(&rest[1..]) as u8),
        Some("attach") => ExitCode::from(cli::attach(&rest[1..]) as u8),
        Some("build") => ExitCode::from(cli::build(&rest[1..]) as u8),
        Some("clean") => ExitCode::from(cli::clean(&rest[1..]) as u8),
        Some("profile") => ExitCode::from(cli::profile(&rest[1..]) as u8),
        Some("pi") => ExitCode::from(cli::pi(&rest[1..]) as u8),
        _ => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
    }
}
