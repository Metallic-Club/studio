//! `cargo xtask perf-report`: checks and tabulates the perf gate's result (`docs/perf.md`).
//!
//! The perf workflow measures the PR base with `--save-baseline=base`, then the head with `--baseline=base
//! --save-summary=json`. gungraun fails the head run when a workload's `Ir` grows past the limit, but it treats a
//! missing baseline as "nothing to compare" and passes. This command closes that hole: every workload listed in
//! the bench's `library_benchmark_group!` must have a head count and a base count, both above zero, compared
//! against the named baseline. It prints the comparison as a Markdown table (the job summary) and fails when a
//! workload is missing, zero, or regressed.
//!
//! The rules are pure functions over strings and JSON values, so they are tested without Valgrind; [`run`] is the
//! thin shell that reads the files.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The bench whose group lists the workloads, relative to the workspace root.
const DEFAULT_SOURCE: &str = "crates/core/benches/perf_gate.rs";
/// The baseline the perf workflow saves the PR base under.
const DEFAULT_BASELINE: &str = "base";
/// gungraun's file name for a benchmark's machine-readable result (`--save-summary=json`).
const SUMMARY_FILE: &str = "summary.json";

/// Command-line options of `perf-report`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// gungraun's output directory (`GUNGRAUN_HOME`), searched for `summary.json` files.
    pub(crate) home: PathBuf,
    /// The bench source whose `benchmarks = [...]` list names the expected workloads.
    pub(crate) source: PathBuf,
    /// The baseline every head result must have been compared against.
    pub(crate) baseline: String,
    /// A file the Markdown table is appended to (the job summary), besides stdout.
    pub(crate) out: Option<PathBuf>,
    /// The suite's name, for the table's title.
    pub(crate) suite: String,
    /// The base has no such suite (the PR introduces it): only head counts are required, and every row is
    /// reported as new, not gated.
    pub(crate) new_suite: bool,
}

/// The suite name when none is given.
const DEFAULT_SUITE: &str = "metallic_core";

/// Parses `perf-report`'s arguments (everything after the command name).
pub(crate) fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut home = None;
    let mut source = PathBuf::from(DEFAULT_SOURCE);
    let mut baseline = DEFAULT_BASELINE.to_owned();
    let mut out = None;
    let mut suite = DEFAULT_SUITE.to_owned();
    let mut new_suite = false;
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = || iter.next().cloned().ok_or_else(|| format!("`{flag}` needs a value"));
        match flag.as_str() {
            "--home" => home = Some(PathBuf::from(value()?)),
            "--source" => source = PathBuf::from(value()?),
            "--baseline" => baseline = value()?,
            "--out" => out = Some(PathBuf::from(value()?)),
            "--suite" => suite = value()?,
            "--new-suite" => new_suite = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    let home = home.ok_or("`--home <gungraun output directory>` is required")?;
    Ok(Options {
        home,
        source,
        baseline,
        out,
        suite,
        new_suite,
    })
}

/// One workload's result, as read from its `summary.json`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measurement {
    /// The benchmark function's name, which is the workload's name.
    pub(crate) workload: String,
    /// The baseline the head was compared against, if any.
    pub(crate) baseline: Option<String>,
    /// The head's instruction count.
    pub(crate) head: Option<u64>,
    /// The base's instruction count.
    pub(crate) base: Option<u64>,
    /// The limit gungraun reported as exceeded, in percent, if the workload regressed.
    pub(crate) regressed_over: Option<f64>,
}

/// What the gate concludes for one workload.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Status {
    /// Measured on both sides and within the limit.
    Pass,
    /// Grew past the limit (in percent).
    Regressed(f64),
    /// Not measured, or not compared; the text says what is missing.
    Missing(String),
    /// A count of zero: the measured function was not found or did nothing.
    Zero,
    /// Measured on the head of a suite the base does not have yet: reported, not gated.
    New,
}

/// One row of the comparison table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    /// The workload's name.
    pub(crate) workload: String,
    /// The base's instruction count, if measured.
    pub(crate) base: Option<u64>,
    /// The head's instruction count, if measured.
    pub(crate) head: Option<u64>,
    /// The verdict.
    pub(crate) status: Status,
}

impl Row {
    /// Whether this row fails the gate.
    pub(crate) fn fails(&self) -> bool {
        !matches!(self.status, Status::Pass | Status::New)
    }
}

/// Runs the check on the files named by `args` and prints the table.
///
/// Returns `Ok(true)` when every workload passes, `Ok(false)` when the gate fails, and `Err` when the inputs
/// could not be read at all.
pub(crate) fn run(args: &[String]) -> Result<bool, String> {
    let options = parse_args(args)?;
    let source =
        fs::read_to_string(&options.source).map_err(|e| format!("could not read {}: {e}", options.source.display()))?;
    let expected = workloads(&source)?;

    let mut files = Vec::new();
    collect_summaries(&options.home, &mut files)?;
    files.sort();
    let mut measurements = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).map_err(|e| format!("could not read {}: {e}", file.display()))?;
        let json: Value =
            serde_json::from_str(&text).map_err(|e| format!("could not parse {}: {e}", file.display()))?;
        measurements.push(parse_summary(&json).map_err(|e| format!("{}: {e}", file.display()))?);
    }

    let rows = if options.new_suite {
        new_rows(&expected, &measurements)
    } else {
        rows(&expected, &measurements, &options.baseline)
    };
    let table = markdown(&rows, &options.baseline, &options.suite, options.new_suite);
    print!("{table}");
    if let Some(out) = &options.out {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)
            .map_err(|e| format!("could not open {}: {e}", out.display()))?;
        file.write_all(table.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", out.display()))?;
    }

    let failed = rows.iter().filter(|r| r.fails()).count();
    if failed > 0 {
        eprintln!("perf-report: {failed} of {} workload(s) failed the gate", rows.len());
    }
    Ok(failed == 0)
}

/// Collects every `summary.json` below `dir`.
fn collect_summaries(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("could not list {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not list {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("could not stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            collect_summaries(&path, out)?;
        } else if file_type.is_file() && entry.file_name() == SUMMARY_FILE {
            out.push(path);
        }
    }
    Ok(())
}

/// The workload names in the bench source's `benchmarks = [...]` list, in order.
pub(crate) fn workloads(source: &str) -> Result<Vec<String>, String> {
    const KEY: &str = "benchmarks = [";
    let start = source
        .find(KEY)
        .ok_or("the bench source has no `benchmarks = [...]` list")?
        + KEY.len();
    let len = source[start..]
        .find(']')
        .ok_or("the `benchmarks = [...]` list is not closed")?;
    let names: Vec<String> = source[start..start + len]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return Err("the `benchmarks = [...]` list is empty".to_owned());
    }
    Ok(names)
}

/// Reads one benchmark's Callgrind `Ir` counts and regressions from its gungraun summary (schema version 7).
pub(crate) fn parse_summary(json: &Value) -> Result<Measurement, String> {
    let workload = json["function_name"].as_str().ok_or("no `function_name`")?.to_owned();
    let baseline = json["baselines"][1].as_str().map(str::to_owned);
    let callgrind = json["profiles"]
        .as_array()
        .ok_or("no `profiles`")?
        .iter()
        .find(|p| p["tool"] == "Callgrind")
        .ok_or("no Callgrind profile")?;
    let total = &callgrind["data"]["total"];
    let values = &total["metrics"]["Ir"]["values"];
    let regressed_over = total["regressions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("Soft"))
        .find(|soft| soft["metric"]["Callgrind"] == "Ir")
        .map(|soft| number(&soft["limit"]).unwrap_or(f64::NAN));
    Ok(Measurement {
        workload,
        baseline,
        head: count(&values["new"]),
        base: count(&values["old"]),
        regressed_over,
    })
}

/// An instruction count, which gungraun writes as a JSON integer.
fn count(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64))
}

/// A percentage, which gungraun writes as a string to keep infinities.
fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

/// The verdict for each expected workload, in the bench's order. A measurement no workload expects is ignored.
pub(crate) fn rows(expected: &[String], measurements: &[Measurement], baseline: &str) -> Vec<Row> {
    expected
        .iter()
        .map(|workload| {
            let found: Vec<&Measurement> = measurements.iter().filter(|m| &m.workload == workload).collect();
            let (base, head, status) = match found.as_slice() {
                [] => (None, None, Status::Missing("no measurement".to_owned())),
                [m] => (m.base, m.head, verdict(m, baseline)),
                _ => (None, None, Status::Missing(format!("{} measurements", found.len()))),
            };
            Row {
                workload: workload.clone(),
                base,
                head,
                status,
            }
        })
        .collect()
}

/// The rows of a suite the base does not have: every expected workload needs one head count above zero, and is
/// reported as new. Nothing is compared, so nothing can regress.
pub(crate) fn new_rows(expected: &[String], measurements: &[Measurement]) -> Vec<Row> {
    expected
        .iter()
        .map(|workload| {
            let found: Vec<&Measurement> = measurements.iter().filter(|m| &m.workload == workload).collect();
            let (head, status) = match found.as_slice() {
                [] => (None, Status::Missing("no measurement".to_owned())),
                [m] => match m.head {
                    None => (None, Status::Missing("no head count".to_owned())),
                    Some(0) => (Some(0), Status::Zero),
                    Some(head) => (Some(head), Status::New),
                },
                _ => (None, Status::Missing(format!("{} measurements", found.len()))),
            };
            Row {
                workload: workload.clone(),
                base: None,
                head,
                status,
            }
        })
        .collect()
}

fn verdict(m: &Measurement, baseline: &str) -> Status {
    if m.baseline.as_deref() != Some(baseline) {
        let against = m.baseline.as_deref().unwrap_or("nothing");
        return Status::Missing(format!("compared against {against}, not baseline `{baseline}`"));
    }
    match (m.base, m.head) {
        (None, _) => Status::Missing("no base count".to_owned()),
        (_, None) => Status::Missing("no head count".to_owned()),
        (Some(0), _) | (_, Some(0)) => Status::Zero,
        _ => match m.regressed_over {
            Some(limit) => Status::Regressed(limit),
            None => Status::Pass,
        },
    }
}

/// The comparison as a Markdown section.
pub(crate) fn markdown(rows: &[Row], baseline: &str, suite: &str, new_suite: bool) -> String {
    let failed = rows.iter().filter(|r| r.fails()).count();
    let mut out = format!("## Perf gate: `{suite}` instruction counts (Callgrind `Ir`)\n\n");
    let verdict = if failed == 0 {
        "**Pass.**".to_owned()
    } else {
        format!("**Fail: {failed} of {} workload(s).**", rows.len())
    };
    if new_suite {
        let _ = writeln!(
            out,
            "The base has no `{suite}` suite yet, so the head's counts are recorded and not gated; the next PR \
             compares against them. {verdict}\n"
        );
    } else {
        let _ = writeln!(
            out,
            "Base and head measured on the same runner with the head's workloads (baseline `{baseline}`). {verdict}\n"
        );
    }
    out.push_str("| Workload | Base | Head | Change | Result |\n");
    out.push_str("| --- | ---: | ---: | ---: | --- |\n");
    for row in rows {
        let change = match (row.base, row.head) {
            (Some(base), Some(head)) if base > 0 => {
                format!("{:+.2}%", (head as f64 - base as f64) / base as f64 * 100.0)
            }
            _ => "n/a".to_owned(),
        };
        let result = match &row.status {
            Status::Pass => "pass".to_owned(),
            Status::Regressed(limit) => format!("**regressed** (limit +{limit}%)"),
            Status::Missing(what) => format!("**missing**: {what}"),
            Status::Zero => "**zero count**: the measured function was not found".to_owned(),
            Status::New => "NEW (not gated)".to_owned(),
        };
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {change} | {result} |",
            row.workload,
            thousands(row.base),
            thousands(row.head)
        );
    }
    out.push('\n');
    out
}

/// `1234567` as `1,234,567`; `None` as `n/a`.
fn thousands(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "n/a".to_owned();
    };
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// A summary as gungraun 0.20 writes it, trimmed to the fields the report reads.
    fn summary(function: &str, baseline: Option<&str>, new: Value, old: Value, regressions: Value) -> Value {
        json!({
            "baselines": [null, baseline],
            "function_name": function,
            "profiles": [{
                "tool": "Callgrind",
                "data": {
                    "parts": [],
                    "total": {
                        "metrics": { "Ir": { "change": null, "values": { "new": new, "old": old } } },
                        "regressions": regressions
                    }
                }
            }]
        })
    }

    fn measured(function: &str, base: u64, head: u64) -> Measurement {
        Measurement {
            workload: function.to_owned(),
            baseline: Some("base".to_owned()),
            head: Some(head),
            base: Some(base),
            regressed_over: None,
        }
    }

    #[test]
    fn workloads_come_from_the_group_list() {
        let source = "library_benchmark_group!(\n    name = workloads,\n    benchmarks = [reads, writes,\n clone]\n);";
        assert_eq!(workloads(source), Ok(names(&["reads", "writes", "clone"])));
        assert!(workloads("main!()").is_err());
        assert!(workloads("benchmarks = [ ]").is_err());
    }

    #[test]
    fn the_real_benches_parse() {
        let source = include_str!("../../crates/core/benches/perf_gate.rs");
        let found = workloads(source);
        assert!(
            found.as_ref().is_ok_and(|w| w.iter().any(|w| w == "writes")),
            "{found:?}"
        );
        let luau = workloads(include_str!("../../crates/luau/benches/perf_gate.rs"));
        assert_eq!(luau, Ok(names(&["luau_reads", "luau_writes", "luau_method_calls"])));
        let render = workloads(include_str!("../../crates/render/benches/perf_gate.rs"));
        assert_eq!(
            render,
            Ok(names(&["render_sync_moving", "render_sync_scattered", "render_cull"]))
        );
        let engine = workloads(include_str!("../../crates/engine/benches/perf_gate.rs"));
        assert_eq!(engine, Ok(names(&["engine_empty_frames"])));
        let client = workloads(include_str!("../../crates/engine/benches/perf_gate_client.rs"));
        assert_eq!(client, Ok(names(&["engine_input_mouse_unwatched"])));
    }

    #[test]
    fn a_new_suite_needs_head_counts_only() {
        let expected = names(&["a", "b", "c", "d"]);
        let mut unmeasured = measured("b", 0, 0);
        unmeasured.head = None;
        unmeasured.base = None;
        let mut zero = measured("c", 0, 0);
        zero.base = None;
        let mut fine = measured("a", 0, 10);
        fine.base = None;
        fine.baseline = None;
        let rows = new_rows(&expected, &[fine, unmeasured, zero]);
        let statuses: Vec<&Status> = rows.iter().map(|r| &r.status).collect();
        assert_eq!(statuses[0], &Status::New);
        assert_eq!(statuses[1], &Status::Missing("no head count".to_owned()));
        assert_eq!(statuses[2], &Status::Zero);
        assert_eq!(statuses[3], &Status::Missing("no measurement".to_owned()));
        assert!(!rows[0].fails());
        assert_eq!(rows.iter().filter(|r| r.fails()).count(), 3);
        let table = markdown(&rows[..1], "base", "metallic_luau", true);
        assert!(table.contains("| `a` | n/a | 10 | n/a | NEW (not gated) |"), "{table}");
        assert!(table.contains("`metallic_luau`"), "{table}");
    }

    #[test]
    fn parses_counts_and_regressions() {
        let regression = json!([{ "Soft": {
            "metric": { "Callgrind": "Ir" }, "new": 110, "old": 100, "diff_pct": "10", "limit": "5"
        } }]);
        let m = parse_summary(&summary("writes", Some("base"), json!(110), json!(100), regression));
        assert_eq!(
            m,
            Ok(Measurement {
                workload: "writes".to_owned(),
                baseline: Some("base".to_owned()),
                head: Some(110),
                base: Some(100),
                regressed_over: Some(5.0),
            })
        );

        let m = parse_summary(&summary("reads", None, json!(7), Value::Null, json!([])));
        assert_eq!(
            m.as_ref().map(|m| (m.base, m.head, m.baseline.clone())),
            Ok((None, Some(7), None))
        );
        assert!(parse_summary(&json!({ "function_name": "x", "profiles": [] })).is_err());
    }

    #[test]
    fn every_failure_kind_fails() {
        let expected = names(&["reads", "writes", "create", "destroy", "clone", "drain_changes"]);
        let mut regressed = measured("writes", 100, 110);
        regressed.regressed_over = Some(5.0);
        let mut unbased = measured("create", 100, 100);
        unbased.base = None;
        let mut wrong_baseline = measured("clone", 100, 100);
        wrong_baseline.baseline = None;
        let measurements = [
            measured("reads", 100, 104),
            regressed,
            unbased,
            measured("destroy", 0, 100),
            wrong_baseline,
            measured("unexpected", 1, 1),
        ];
        let rows = rows(&expected, &measurements, "base");
        let statuses: Vec<&Status> = rows.iter().map(|r| &r.status).collect();
        assert_eq!(statuses[0], &Status::Pass);
        assert_eq!(statuses[1], &Status::Regressed(5.0));
        assert_eq!(statuses[2], &Status::Missing("no base count".to_owned()));
        assert_eq!(statuses[3], &Status::Zero);
        assert!(matches!(statuses[4], Status::Missing(_)));
        assert_eq!(statuses[5], &Status::Missing("no measurement".to_owned()));
        assert_eq!(rows.iter().filter(|r| r.fails()).count(), 5);
    }

    #[test]
    fn duplicate_measurements_fail() {
        let rows = rows(
            &names(&["reads"]),
            &[measured("reads", 1, 1), measured("reads", 1, 1)],
            "base",
        );
        assert!(rows[0].fails());
    }

    #[test]
    fn table_shows_counts_and_change() {
        let rows = rows(&names(&["reads"]), &[measured("reads", 1_000_000, 1_050_000)], "base");
        let table = markdown(&rows, "base", "metallic_core", false);
        assert!(
            table.contains("| `reads` | 1,000,000 | 1,050,000 | +5.00% | pass |"),
            "{table}"
        );
        assert!(table.contains("**Pass.**"));
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(Some(0)), "0");
        assert_eq!(thousands(Some(999)), "999");
        assert_eq!(thousands(Some(1_000)), "1,000");
        assert_eq!(thousands(Some(12_345_678)), "12,345,678");
        assert_eq!(thousands(None), "n/a");
    }

    #[test]
    fn options() {
        let args = names(&["--home", "h", "--baseline", "b", "--out", "o"]);
        let options = parse_args(&args);
        assert_eq!(
            options,
            Ok(Options {
                home: PathBuf::from("h"),
                source: PathBuf::from(DEFAULT_SOURCE),
                baseline: "b".to_owned(),
                out: Some(PathBuf::from("o")),
                suite: DEFAULT_SUITE.to_owned(),
                new_suite: false,
            })
        );
        let new = parse_args(&names(&["--home", "h", "--suite", "metallic_luau", "--new-suite"]));
        assert!(
            new.as_ref().is_ok_and(|o| o.new_suite && o.suite == "metallic_luau"),
            "{new:?}"
        );
        assert!(parse_args(&names(&["--baseline", "b"])).is_err());
        assert!(parse_args(&names(&["--home"])).is_err());
        assert!(parse_args(&names(&["--bogus"])).is_err());
    }
}
