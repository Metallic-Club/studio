//! Repository automation, run as `cargo xtask <command>`.
//!
//! Commands:
//!
//! - `check-layout` (#11): fails when a crate breaks the dependency rules of `docs/adr/0001-project-layout.md`,
//!   when `metallic_core` gains a dependency outside its allow-list, or when `cfg(target_os ...)` appears outside
//!   `crates/platform` and `apps/`.
//!
//! Later issues add `api-dump` (M2) and `package` (M8).

mod layout;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  check-layout    check crate dependencies, metallic_core's allow-list and cfg(target_os) use
                  against docs/adr/0001-project-layout.md";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("check-layout") => match layout::run() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("check-layout: {e}");
                ExitCode::FAILURE
            }
        },
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
