mod cli;
mod config;
mod core;
mod dirs;
mod init;
mod pi;
mod trust;
mod update;

use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::pi::launch;

fn main() -> ExitCode {
    // The one binary doubles as PID 1 in a box on Linux and as the `pi` shim.
    let mut args = std::env::args_os();
    let program = args.next();
    // A symlink named `pi` makes argv[0] the shim; every argument after it is
    // pi's.
    let shim = program
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("pi");
    let rest: Vec<String> = match args.map(|arg| arg.into_string()).collect() {
        Ok(rest) => rest,
        Err(arg) => {
            eprintln!("pinfold: argument {arg:?} is not valid UTF-8");
            return ExitCode::from(1);
        }
    };
    // The shim is the `pi` verb, so pi's own `--help` and `--version` reach pi.
    let (verb, args) = if shim {
        ("pi", &rest[..])
    } else {
        (
            rest.first().map(String::as_str).unwrap_or_default(),
            rest.get(1..).unwrap_or_default(),
        )
    };
    let run: fn(&[String]) -> io::Result<i32> = match verb {
        // The options answer before any verb runs, `init` included.
        "--version" | "-V" => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        "--help" | "-h" => {
            println!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        // PID 1 never returns, so it never reaches `report`.
        "init" => |args| init::run(args),
        "box" => cli::run,
        "allow" => cli::allow,
        "attach" => cli::attach,
        "build" => cli::build,
        "image" => cli::image,
        "clean" => cli::clean,
        "doctor" => cli::doctor,
        "artifacts" => cli::artifacts,
        "config" => cli::config,
        "profile" => cli::profile,
        "pi" => launch::run,
        "update" => update::run,
        _ => {
            eprintln!("{}", cli::USAGE);
            return ExitCode::from(1);
        }
    };
    // A subcommand's help flag answers too, before `init` and the daily
    // pass, so it touches no runtime and no state dir.
    if cli::help(verb, args) {
        return ExitCode::SUCCESS;
    }
    // Updating and checking releases need neither a runtime nor maintenance.
    if verb == "update" {
        return ExitCode::from(cli::report(verb, run(args)) as u8);
    }
    let _executable = if verb == "init" {
        None
    } else {
        match update::working() {
            Ok(lock) => Some(lock),
            Err(error) => return ExitCode::from(cli::report(verb, Err(error)) as u8),
        }
    };
    update::notice(verb, args);
    // The daily pass runs before any working command, the `pi` shim
    // included; `init` is PID 1 in a box with no runtime to prune, and the
    // options, help and doctor touch nothing. `box up` runs the pass itself,
    // so its SIGTERM and SIGINT handlers come before the pass lists the
    // runtime.
    match (verb, args.first().map(String::as_str)) {
        ("init" | "doctor" | "config" | "artifacts", _) | ("box", Some("up")) => {}
        _ => crate::core::clean::maintain(),
    }
    ExitCode::from(cli::report(verb, run(args)) as u8)
}
