//! Where script output and errors go: the `OutputSink` trait and a collecting sink.

/// Which script a line of output came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputSource<'a> {
    /// The script instance's full name when it started, such as `Workspace.Door.Script`.
    pub script: &'a str,
    /// The chunk the line came from: `path#uid` for a running script, the source path for a compile error.
    pub chunk: &'a str,
    /// The 1-based source line, when known.
    pub line: Option<u32>,
}

/// Receives what scripts print and every script error. Studio's Output panel implements it later; tests use
/// [`CollectOutput`]. A sink never fails: output is best effort and never stops a script.
pub trait OutputSink {
    /// A script printed `message`.
    fn print(&mut self, from: &OutputSource<'_>, message: &str);

    /// A script warned `message`.
    fn warn(&mut self, from: &OutputSource<'_>, message: &str);

    /// A script failed to compile, start or run. `message` includes the traceback, when there is one.
    fn error(&mut self, from: &OutputSource<'_>, message: &str);
}

impl<T: OutputSink + ?Sized> OutputSink for Box<T> {
    fn print(&mut self, from: &OutputSource<'_>, message: &str) {
        (**self).print(from, message);
    }

    fn warn(&mut self, from: &OutputSource<'_>, message: &str) {
        (**self).warn(from, message);
    }

    fn error(&mut self, from: &OutputSource<'_>, message: &str) {
        (**self).error(from, message);
    }
}

/// The kind of an [`OutputLine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputLevel {
    /// From [`OutputSink::print`].
    Print,
    /// From [`OutputSink::warn`].
    Warn,
    /// From [`OutputSink::error`].
    Error,
}

/// One line kept by [`CollectOutput`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    /// Print, warning or error.
    pub level: OutputLevel,
    /// [`OutputSource::script`].
    pub script: String,
    /// [`OutputSource::chunk`].
    pub chunk: String,
    /// [`OutputSource::line`].
    pub line: Option<u32>,
    /// The text.
    pub message: String,
}

/// A sink that keeps every line in memory, for tests and headless tools.
#[derive(Clone, Debug, Default)]
pub struct CollectOutput {
    /// Every line received, in order.
    pub lines: Vec<OutputLine>,
}

impl CollectOutput {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// The lines of one level, in order.
    pub fn of_level(&self, level: OutputLevel) -> impl Iterator<Item = &OutputLine> {
        self.lines.iter().filter(move |line| line.level == level)
    }

    fn push(&mut self, level: OutputLevel, from: &OutputSource<'_>, message: &str) {
        self.lines.push(OutputLine {
            level,
            script: from.script.to_owned(),
            chunk: from.chunk.to_owned(),
            line: from.line,
            message: message.to_owned(),
        });
    }
}

impl OutputSink for CollectOutput {
    fn print(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(OutputLevel::Print, from, message);
    }

    fn warn(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(OutputLevel::Warn, from, message);
    }

    fn error(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(OutputLevel::Error, from, message);
    }
}
