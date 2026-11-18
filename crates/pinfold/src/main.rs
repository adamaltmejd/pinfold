mod cli;
mod config;
mod core;
mod dirs;
use crate::init::formatting;
#[cfg(target_os = "linux")]
mod image_limits;
mod init;
mod pi;
mod update;

use std::io::{self, Write};
use std::process::ExitCode;

use crate::pi::{launch, session};

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    args.next();
    let args: Vec<_> = args.collect();
    let project_command = args
        .first()
        .is_some_and(|arg| arg == "--" || arg == "start" || arg == "stop");
    let rest: Vec<String> = match args.into_iter().map(|arg| arg.into_string()).collect() {
        Ok(rest) => rest,
        Err(arg) => {
            eprintln!("pinfold: argument {arg:?} is not valid UTF-8");
            return ExitCode::from(if project_command { 125 } else { 1 });
        }
    };
    if rest.is_empty() {
        cli::print_help();
        return ExitCode::SUCCESS;
    }
    let verb = rest.first().map(String::as_str).unwrap_or_default();
    let args = rest.get(1..).unwrap_or_default();
    let run: fn(&[String]) -> io::Result<i32> = match verb {
        "--version" | "-V" => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        "--help" | "-h" => {
            cli::print_help();
            return ExitCode::SUCCESS;
        }
        // PID 1 never returns, so it never reaches `report`.
        "init" => |args| init::run(args),
        "box" => cli::run,
        "build" => cli::build,
        "image" => cli::image,
        "clean" => cli::clean,
        "disk" => cli::disk,
        "doctor" => cli::doctor,
        "artifacts" => cli::artifacts,
        "apparmor-profile" => cli::apparmor_profile,
        "config" => cli::config,
        "profile" => cli::profile,
        "--" => session::run,
        "start" => session::start,
        "stop" => session::stop,
        "__project-owner" => session::owner,
        "push" => |args| launch::crossing("push", args),
        "fetch" => |args| launch::crossing("fetch", args),
        "update" => update::run,
        _ => {
            eprintln!("pinfold: unknown command {verb:?}. Run `pinfold --help` for commands.");
            return ExitCode::from(1);
        }
    };
    if cli::help(verb, args) {
        return ExitCode::SUCCESS;
    }
    // `update` takes the executable lock itself, exclusively.
    let _executable = if matches!(
        verb,
        "init" | "update" | "__project-owner" | "--" | "start" | "stop"
    ) {
        None
    } else {
        match update::working() {
            Ok(lock) => Some(lock),
            Err(error) => return ExitCode::from(cli::report(verb, Err(error)) as u8),
        }
    };
    if !matches!(verb, "--" | "start" | "stop" | "__project-owner") {
        update::notice(verb);
    }
    // Project admission and box up install their signal handlers first.
    match (verb, args.first().map(String::as_str)) {
        (
            "init" | "update" | "doctor" | "config" | "artifacts" | "apparmor-profile" | "--"
            | "start" | "stop" | "__project-owner" | "push" | "fetch",
            _,
        )
        | ("box", Some("up" | "list" | "down" | "inspect" | "events")) => {}
        _ => crate::core::clean::maintain(),
    }
    let result = run(args);
    let restored = crate::core::runtime::terminal::restore();
    let result = match restored {
        Ok(_) => result,
        Err(error) => {
            let error = io::Error::other(format!("restore terminal: {error}"));
            if matches!(result, Ok(0)) {
                Err(error)
            } else {
                let _ = writeln!(io::stderr(), "pinfold {verb}: {error}");
                result
            }
        }
    };
    ExitCode::from(cli::report(verb, result) as u8)
}
