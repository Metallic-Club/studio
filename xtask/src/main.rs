//! Repository automation, run as `cargo xtask <command>`.
//!
//! Commands:
//!
//! - `check-layout` (#11): fails when a crate breaks the dependency rules of `docs/adr/0001-project-layout.md`,
//!   when `metallic_core` gains a dependency outside its allow-list, or when `cfg(target_os ...)` appears outside
//!   `crates/platform` and `apps/`.
//! - `perf-report` (#29): after the perf workflow's gungraun runs, fails when a perf-gate workload is missing a
//!   base or head instruction count, has a zero count, or regressed, and prints the comparison table
//!   (`docs/perf.md`).
//! - `api-dump` (#48): writes the scripting API as `api/api-dump.json` and the Luau type definitions
//!   `api/metallic.d.luau`; with `--check`, fails when either committed file differs from what it would write.
//!
//! A later issue adds `package` (M8).

mod api;
mod layout;
mod perf;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <command>

commands:
  api-dump        write api/api-dump.json and api/metallic.d.luau from the class registry
                  --check            write nothing; fail if either committed file is stale
  check-layout    check crate dependencies, metallic_core's allow-list and cfg(target_os) use
                  against docs/adr/0001-project-layout.md
  perf-report     check and tabulate the perf gate's gungraun results (docs/perf.md)
                  --home <dir>       gungraun output directory (required)
                  --source <file>    bench listing the workloads [crates/core/benches/perf_gate.rs]
                  --baseline <name>  baseline the head was compared against [base]
                  --suite <name>     the suite's name in the table's title [metallic_core]
                  --new-suite        the base has no such suite: require head counts only, report NEW
                  --out <file>       also append the Markdown table here (e.g. $GITHUB_STEP_SUMMARY)";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("api-dump") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            match api::run(&args) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("api-dump: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("check-layout") => match layout::run() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("check-layout: {e}");
                ExitCode::FAILURE
            }
        },
        Some("perf-report") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            match perf::run(&args) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("perf-report: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
