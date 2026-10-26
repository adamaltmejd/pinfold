#[path = "../../pinfold/src/init.rs"]
mod init;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string())
        .collect()
    {
        Ok(args) => args,
        Err(arg) => {
            eprintln!("pinfold: argument {arg:?} is not valid UTF-8");
            return ExitCode::from(1);
        }
    };
    match args.first().map(String::as_str) {
        Some("init") => init::run(&args[1..]),
        _ => {
            eprintln!("usage: pinfold init");
            ExitCode::from(1)
        }
    }
}
