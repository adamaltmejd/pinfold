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

use crate::pi::launch;

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    args.next();
    let args: Vec<_> = args.collect();
    let project_command = args.first().is_some_and(|arg| arg == "--");
    let rest: Vec<String> = match args.into_iter().map(|arg| arg.into_string()).collect() {
        Ok(rest) => rest,
        Err(arg) => {
            eprintln!("pinfold: argument {arg:?} is not valid UTF-8");
            return ExitCode::from(if project_command { 125 } else { 1 });
        }
    };
    if rest.is_empty() {
        println!("{}", cli::USAGE);
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
            println!("{}", cli::USAGE);
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
        "--" => launch::run,
        "push" => |args| launch::crossing("push", args),
        "fetch" => |args| launch::crossing("fetch", args),
        "update" => update::run,
        _ => {
            eprintln!("{}", cli::USAGE);
            return ExitCode::from(1);
        }
    };
    if cli::help(verb, args) {
        return ExitCode::SUCCESS;
    }
    // `update` takes the executable lock itself, exclusively.
    let _executable = if matches!(verb, "init" | "update") {
        None
    } else {
        match update::working() {
            Ok(lock) => Some(lock),
            Err(error) => return ExitCode::from(cli::report(verb, Err(error)) as u8),
        }
    };
    update::notice(verb);
    // The daily pass runs before working commands, project commands included.
    // `box up` runs it itself, after installing its signal handlers.
    match (verb, args.first().map(String::as_str)) {
        ("init" | "update" | "doctor" | "config" | "artifacts" | "apparmor-profile", _)
        | ("box", Some("up" | "list" | "down" | "inspect" | "events")) => {}
        _ => crate::core::clean::maintain(),
    }
    #[cfg(not(target_os = "linux"))]
    let stdin_flags =
        if verb == "--" || (verb == "box" && args.first().is_some_and(|arg| arg == "exec")) {
            match crate::core::runtime::apple::StdinFlags::save() {
                Ok(flags) => flags,
                Err(error) => return ExitCode::from(cli::report(verb, Err(error)) as u8),
            }
        } else {
            None
        };
    let result = run(args);
    let restored = crate::core::runtime::terminal::restore();
    #[cfg(not(target_os = "linux"))]
    let restored = restored.and(stdin_flags.map(|flags| flags.restore()).transpose());
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
