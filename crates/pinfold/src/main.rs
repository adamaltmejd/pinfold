use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use pinfold::pi::launch;
use pinfold::{cli, init};

/// One line per verb from the CLI table in docs/ARCHITECTURE.md, plus the
/// options that answer before a verb is chosen. Printed on stdout by
/// `--help` and on stderr for a malformed invocation.
const USAGE: &str = "\
pinfold pi [pi args…]            pi in a box for this project; `pi` is a symlink to this
pinfold attach [--box NAME] [cmd…]   bash (or cmd) in this project's running pi box
pinfold build [--profile NAME]   build this project's image, or a profile's; prints the ref
pinfold image build NAME --containerfile PATH --context DIR [--label KEY=VALUE]... [--no-cache]   a caller's image from its own context; one JSON line
pinfold allow                    trust this project's .pinfold.toml and Containerfile
pinfold profile new NAME [--from PROFILE] [--from-project [PATH]]   copy a profile to edit as files
pinfold clean [--dry-run] [--unused AGE]   reclaim disk (see Maintenance)
pinfold doctor                   runtime, kernel, image, artifacts, trust, config, disk use
pinfold artifacts                the pinned artifacts as JSON: name, version, sha256, path, cached
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
    let verb = rest
        .first()
        .and_then(|verb| verb.to_str())
        .unwrap_or_default();
    // The daily pass runs before any command, the `pi` shim included. Only a
    // verb that does work pays for it: `init` is PID 1 in a box with the
    // project home for state and no runtime to prune, and `--version`,
    // `--help`, a bare `pinfold` and an unknown verb touch nothing.
    let working = matches!(
        verb,
        "box"
            | "build"
            | "image"
            | "pi"
            | "clean"
            | "doctor"
            | "artifacts"
            | "config"
            | "profile"
            | "allow"
            | "attach"
    );
    if shim || working {
        pinfold::core::clean::maintain();
    }
    if shim {
        return ExitCode::from(cli::report("pi", launch::run(&rest)) as u8);
    }
    let args = rest.get(1..).unwrap_or_default();
    // PID 1 never returns, so it never reaches `report`.
    if verb == "init" {
        init::run(args)
    }
    let result = match verb {
        "--version" | "-V" => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        "box" => cli::run(args),
        "allow" => cli::allow(args),
        "attach" => cli::attach(args),
        "build" => cli::build(args),
        "image" => cli::image(args),
        "clean" => cli::clean(args),
        "doctor" => cli::doctor(args),
        "artifacts" => cli::artifacts(args),
        "config" => cli::config(args),
        "profile" => cli::profile(args),
        "pi" => launch::run(args),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(1);
        }
    };
    ExitCode::from(cli::report(verb, result) as u8)
}
