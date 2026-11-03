//! Repository automation, run as `cargo xtask <command>`.
//!
//! Commands are added by the issues that need them (`check-layout` in #11, `api-dump` in M2, `package` in M8).

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <command>\n\ncommands: (none yet)";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
