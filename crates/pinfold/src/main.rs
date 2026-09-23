use std::process::ExitCode;

use pinfold::{cli, init};

fn main() -> ExitCode {
    // The one binary doubles as PID 1 in a box on Linux.
    let mut args = std::env::args_os();
    args.next();
    let verb = args.next();
    // The daily pass runs before any command but never as PID 1 in a box,
    // which has the project home for state and no runtime to prune.
    if verb.as_deref().and_then(|verb| verb.to_str()) != Some("init") {
        pinfold::core::clean::maintain();
    }
    match verb.as_deref().and_then(|verb| verb.to_str()) {
        Some("init") => init::run(&args.collect::<Vec<_>>()),
        Some("box") => ExitCode::from(cli::run(&args.collect::<Vec<_>>()) as u8),
        Some("build") => ExitCode::from(cli::build(&args.collect::<Vec<_>>()) as u8),
        _ => {
            println!("pinfold {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
    }
}
