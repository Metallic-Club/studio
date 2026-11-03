//! [`StdOutput`]: script output on the process's standard streams, the default for servers and examples.

use std::io::Write;

use metallic_script::{OutputSink, OutputSource};

/// Writes what scripts print to stdout, and warnings and errors (with the script's name and line) to stderr.
///
/// The default output of [`Engine::new`](crate::Engine::new): what a dedicated server or a command-line tool wants.
/// Studio and the Player plug their own [`OutputSink`] in with [`Engine::with_output`](crate::Engine::with_output).
/// Like every sink it never fails: a closed stream drops the line.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdOutput;

impl OutputSink for StdOutput {
    fn print(&mut self, _from: &OutputSource<'_>, message: &str) {
        let _ = writeln!(std::io::stdout().lock(), "{message}");
    }

    fn warn(&mut self, from: &OutputSource<'_>, message: &str) {
        let _ = writeln!(std::io::stderr().lock(), "warning: {}: {message}", from.script);
    }

    fn error(&mut self, from: &OutputSource<'_>, message: &str) {
        let mut stderr = std::io::stderr().lock();
        let _ = match from.line {
            Some(line) => writeln!(stderr, "error: {}:{line}: {message}", from.script),
            None => writeln!(stderr, "error: {}: {message}", from.script),
        };
    }
}
