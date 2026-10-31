//! The script debugger in Studio (#22): breakpoints in the script editor's gutter, pause and
//! step during Play, and the Call Stack, Variables and Watch panes.
//!
//! Breakpoints live in [`DebugSettings`] on the [`EditorState`], so they survive Stop and apply
//! from the first line of the next Play ([`crate::play::PlayStart::debug`]). While a session runs
//! they are also sent to its game through [`crate::play::PlayView::debugger`]. The game runs on
//! the UI thread and a paused script *breaks* instead of blocking, so Studio stays usable while
//! paused: the viewport, Explorer and Properties keep working.
//!
//! Keys: F5 continues when paused (it is Play otherwise), F9 toggles a breakpoint on the script
//! editor's cursor line, F10 steps over, F11 into, Shift+F11 out.

mod call_stack;
mod gutter;
mod variables;
mod watch;

use std::collections::{BTreeMap, HashMap};

use eframe::egui::{self, RichText};
use metallic_core::InstanceId;
use metallic_script::{Breakpoint, DebugState, Debugger, PauseReason, Scope, VarHandle, VarNode};

use crate::actions::{Action, ActionRegistry, ToolbarGroup};
use crate::editor::EditorState;
use crate::icons::Icon;
use crate::tokens::ui_tokens;
use crate::widgets::button::Button;

pub use gutter::{Gutter, LineRow, gutter, hover, line_rows, name_at};

/// Breakpoints and options of the script debugger, kept with the edit session.
#[derive(Debug, Default, Clone)]
pub struct DebugSettings {
    /// Breakpoints by script and line.
    pub breakpoints: BTreeMap<InstanceId, BTreeMap<u32, Breakpoint>>,
    /// Pause at errors that no `pcall` handles.
    pub break_on_error: bool,
    /// Bumped by every debugger action, so cached views refresh.
    generation: u64,
}

impl DebugSettings {
    /// Sends every breakpoint and option to `debugger` (a game about to start).
    pub fn apply(&self, debugger: &mut Debugger<'_>) {
        for (&script, lines) in &self.breakpoints {
            for (&line, kind) in lines {
                debugger.set_breakpoint(script, line, kind.clone());
            }
        }
        debugger.break_on_error(self.break_on_error);
    }

    /// The breakpoint on `line` of `script`.
    pub fn get(&self, script: InstanceId, line: u32) -> Option<&Breakpoint> {
        self.breakpoints.get(&script)?.get(&line)
    }

    /// Changes every time the debugger acts (step, continue, breakpoint edits).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl EditorState {
    /// The running game's debugger (`None` outside an in-Studio Play).
    pub fn debugger(&mut self) -> Option<Debugger<'_>> {
        self.play.view_mut()?.debugger()
    }

    /// True while the running game is paused in the debugger.
    pub fn is_paused(&self) -> bool {
        self.play.view().is_some_and(|v| v.is_paused())
    }

    /// Sets (`Some`) or removes (`None`) the breakpoint on `line` of `script`, now and for
    /// later Play sessions.
    pub fn set_breakpoint(&mut self, script: InstanceId, line: u32, kind: Option<Breakpoint>) {
        let lines = self.debug.breakpoints.entry(script).or_default();
        match &kind {
            Some(k) => {
                lines.insert(line, k.clone());
            }
            None => {
                lines.remove(&line);
            }
        }
        if lines.is_empty() {
            self.debug.breakpoints.remove(&script);
        }
        if let Some(mut d) = self.debugger() {
            match kind {
                Some(k) => d.set_breakpoint(script, line, k),
                None => d.clear_breakpoint(script, line),
            }
        }
        self.debug.generation += 1;
    }

    /// Adds a plain breakpoint on `line` of `script`, or removes the one there.
    pub fn toggle_breakpoint(&mut self, script: InstanceId, line: u32) {
        let kind = match self.debug.get(script, line) {
            Some(_) => None,
            None => Some(Breakpoint::Plain),
        };
        self.set_breakpoint(script, line, kind);
    }

    /// Turns break-on-error on or off.
    pub fn set_break_on_error(&mut self, on: bool) {
        self.debug.break_on_error = on;
        if let Some(mut d) = self.debugger() {
            d.break_on_error(on);
        }
        self.debug.generation += 1;
    }

    /// Runs `act` on the game's debugger and marks views stale.
    pub fn debug_act(&mut self, act: impl FnOnce(&mut Debugger<'_>)) {
        if let Some(mut d) = self.debugger() {
            act(&mut d);
        }
        self.debug.generation += 1;
    }
}

/// A debugger command run by an action.
type DebugAct = fn(&mut Debugger<'_>);

/// Registers the debugger's actions (`debug.*`; F9 is handled by the editor UI, which knows the
/// script editor's cursor).
pub fn register(r: &mut ActionRegistry) {
    r.register(Action::new(
        "debug.toggle_breakpoint",
        "Toggle Breakpoint",
        Icon::Text(""),
        ToolbarGroup::Hidden,
        |_| {},
    ));
    let paused = |s: &EditorState| s.is_paused();
    let actions: [(&'static str, &str, DebugAct); 4] = [
        ("debug.continue", "Continue", |d| d.resume()),
        ("debug.step_over", "Step Over", |d| d.step_over()),
        ("debug.step_into", "Step Into", |d| d.step_into()),
        ("debug.step_out", "Step Out", |d| d.step_out()),
    ];
    for (id, label, act) in actions {
        r.register(
            Action::new(id, label, Icon::Text(""), ToolbarGroup::Hidden, move |s| {
                s.debug_act(act)
            })
            .enabled(paused),
        );
    }
    r.register(
        Action::new(
            "debug.pause",
            "Pause",
            Icon::Text(""),
            ToolbarGroup::Hidden,
            |s| s.debug_act(|d| d.pause()),
        )
        .enabled(|s| s.play.view().is_some() && !s.is_paused()),
    );
}

/// A breakpoint whose condition or log message is being edited.
struct Editing {
    script: InstanceId,
    line: u32,
    log: bool,
    text: String,
}

/// A local whose new value is being typed.
struct SetLocal {
    name: String,
    text: String,
    error: Option<String>,
}

/// The debugger pane shown under the viewport during Play, and the state behind the script
/// editor's paused-line marker and hover values.
#[derive(Default)]
pub struct DebugPanel {
    /// The selected call stack frame (0 = innermost).
    pub frame: usize,
    /// Watch expressions.
    pub watches: Vec<String>,
    new_watch: String,
    /// The debugger state when last read.
    state: Option<DebugState>,
    seen: Option<u64>,
    was_paused: bool,
    vars: HashMap<(usize, u8), Vec<VarNode>>,
    expanded: HashMap<VarHandle, Vec<VarNode>>,
    values: HashMap<String, Result<String, String>>,
    editing: Option<Editing>,
    setting: Option<SetLocal>,
    /// A script to show at a line (the paused frame), taken by the editor UI.
    reveal: Option<(InstanceId, u32)>,
}

impl DebugPanel {
    /// A pane with no watches.
    pub fn new() -> DebugPanel {
        DebugPanel::default()
    }

    /// Reads the game's debugger state when it changed. On a new pause: a line in Output and a
    /// request to show the paused script ([`DebugPanel::take_reveal`]).
    pub fn poll(&mut self, state: &mut EditorState) {
        let paused = state.is_paused();
        if paused != self.was_paused {
            self.was_paused = paused;
            state.debug.generation += 1;
        }
        if self.seen == Some(state.debug.generation) {
            return;
        }
        self.seen = Some(state.debug.generation);
        self.vars.clear();
        self.expanded.clear();
        self.values.clear();
        let before = self.state.take();
        self.state = state.debugger().map(|d| d.state());
        let was = matches!(before, Some(DebugState::Paused { .. }));
        if let Some(DebugState::Paused { reason, stack }) = &self.state {
            if self.frame >= stack.len() {
                self.frame = 0;
            }
            if !was {
                self.frame = 0;
            }
            if let Some(top) = stack.first() {
                if let Some(script) = top.script {
                    self.reveal = Some((script, top.line));
                }
                state.output.studio(format!(
                    "Paused: {} at {}:{}",
                    reason_text(reason),
                    top.chunk,
                    top.line
                ));
            } else if !was {
                state.output.studio("Paused");
            }
        } else {
            self.frame = 0;
        }
    }

    /// The script and line to show after a pause or a frame click, once.
    pub fn take_reveal(&mut self) -> Option<(InstanceId, u32)> {
        self.reveal.take()
    }

    /// The paused line of `script` in the selected frame, for the editor's marker.
    pub fn paused_line(&self, script: InstanceId) -> Option<u32> {
        match &self.state {
            Some(DebugState::Paused { stack, .. }) => stack
                .get(self.frame)
                .filter(|f| f.script == Some(script))
                .map(|f| f.line),
            _ => None,
        }
    }

    /// True while paused (as of the last [`DebugPanel::poll`]).
    pub fn is_paused(&self) -> bool {
        matches!(self.state, Some(DebugState::Paused { .. }))
    }

    /// The value of `expr` in the selected frame, for hover tips (cached until the game runs).
    pub fn hover_value(&mut self, state: &mut EditorState, expr: &str) -> Option<String> {
        if !self.is_paused() {
            return None;
        }
        let frame = self.frame;
        self.value(state, frame, expr).ok()
    }

    fn value(
        &mut self,
        state: &mut EditorState,
        frame: usize,
        expr: &str,
    ) -> Result<String, String> {
        let key = format!("{frame}\u{0}{expr}");
        if let Some(v) = self.values.get(&key) {
            return v.clone();
        }
        let v = match state.debugger() {
            Some(d) => d.evaluate(frame, expr).map_err(|e| e.message),
            None => Err("not playing".into()),
        };
        self.values.insert(key, v.clone());
        v
    }

    fn scope(
        &mut self,
        state: &mut EditorState,
        frame: usize,
        scope: Scope,
        key: u8,
    ) -> Vec<VarNode> {
        if let Some(v) = self.vars.get(&(frame, key)) {
            return v.clone();
        }
        let v = state
            .debugger()
            .map(|d| d.variables(frame, scope))
            .unwrap_or_default();
        self.vars.insert((frame, key), v.clone());
        v
    }

    fn begin_set_local(&mut self, name: &str, display: &str) {
        self.setting = Some(SetLocal {
            name: name.to_string(),
            text: display.to_string(),
            error: None,
        });
    }

    /// Draws the Set value window for a local, if open.
    fn set_local_window(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let Some(set) = &mut self.setting else {
            return;
        };
        let mut done = None;
        egui::Window::new(format!("Set {}", set.name))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("A Luau expression, evaluated in this frame.")
                        .color(ui_tokens().text_dim),
                );
                let r = ui.add(
                    egui::TextEdit::singleline(&mut set.text)
                        .font(ui_tokens().mono())
                        .desired_width(320.0),
                );
                r.request_focus();
                if let Some(e) = &set.error {
                    ui.label(RichText::new(e).color(ui_tokens().error));
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.add(Button::primary("Set")).clicked() || enter {
                        done = Some(true);
                    }
                    if ui.add(Button::ghost("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                let frame = self.frame;
                let result = match state.debugger() {
                    Some(mut d) => d
                        .set_local(frame, &set.name, &set.text)
                        .map_err(|e| e.message),
                    None => Err("not playing".into()),
                };
                match result {
                    Ok(()) => {
                        self.setting = None;
                        state.debug.generation += 1;
                    }
                    Err(e) => set.error = Some(e),
                }
            }
            Some(false) => self.setting = None,
            None => {}
        }
    }

    fn expand(&mut self, state: &mut EditorState, handle: VarHandle) -> Vec<VarNode> {
        if let Some(v) = self.expanded.get(&handle) {
            return v.clone();
        }
        let v = state
            .debugger()
            .map(|d| d.expand(handle))
            .unwrap_or_default();
        self.expanded.insert(handle, v.clone());
        v
    }

    /// Opens the condition or log message editor for a breakpoint (gutter right-click).
    pub fn edit_breakpoint(
        &mut self,
        state: &EditorState,
        script: InstanceId,
        line: u32,
        log: bool,
    ) {
        let text = match (state.debug.get(script, line), log) {
            (Some(Breakpoint::Condition(c)), false) => c.clone(),
            (Some(Breakpoint::Log(m)), true) => m.clone(),
            _ => String::new(),
        };
        self.editing = Some(Editing {
            script,
            line,
            log,
            text,
        });
    }

    /// Draws the breakpoint editor window, if open.
    pub fn edit_window(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let Some(edit) = &mut self.editing else {
            return;
        };
        let title = if edit.log {
            format!("Log message, line {}", edit.line)
        } else {
            format!("Condition, line {}", edit.line)
        };
        let mut done = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(if edit.log {
                        "Printed when the line runs, without pausing. {expr} shows a value."
                    } else {
                        "Pauses only when this Luau expression is true."
                    })
                    .color(ui_tokens().text_dim),
                );
                let r = ui.add(
                    egui::TextEdit::singleline(&mut edit.text)
                        .font(ui_tokens().mono())
                        .desired_width(360.0),
                );
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if ui.add(Button::primary("Save")).clicked() || enter {
                        done = Some(true);
                    }
                    if ui.add(Button::ghost("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        done = Some(false);
                    }
                });
            });
        // Cancel takes the edit too: the chain runs left to right.
        if let Some(save) = done
            && let Some(edit) = self.editing.take()
            && save
        {
            let text = edit.text.trim().to_string();
            let kind = match (text.is_empty(), edit.log) {
                (true, _) => Breakpoint::Plain,
                (false, true) => Breakpoint::Log(text),
                (false, false) => Breakpoint::Condition(text),
            };
            state.set_breakpoint(edit.script, edit.line, Some(kind));
        }
    }

    /// Draws the pane: status and step buttons, then Call Stack, Variables and Watch.
    pub fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        self.poll(state);
        let paused = self.is_paused();
        ui.horizontal(|ui| {
            let status = match &self.state {
                Some(DebugState::Paused { reason, stack }) => match stack.first() {
                    Some(f) => {
                        format!("Paused ({}) at {}:{}", reason_text(reason), f.chunk, f.line)
                    }
                    None => "Paused".to_string(),
                },
                _ => "Running".to_string(),
            };
            ui.label(RichText::new(status).color(if paused {
                ui_tokens().warning
            } else {
                ui_tokens().text_dim
            }));
            ui.add_space(8.0);
            let mut run = None;
            if paused {
                if ui
                    .add(Button::secondary("Continue").keycap("F5").compact())
                    .clicked()
                {
                    run = Some("debug.continue");
                }
                if ui
                    .add(Button::ghost("Step Over").keycap("F10").compact())
                    .clicked()
                {
                    run = Some("debug.step_over");
                }
                if ui
                    .add(Button::ghost("Step Into").keycap("F11").compact())
                    .clicked()
                {
                    run = Some("debug.step_into");
                }
                if ui
                    .add(Button::ghost("Step Out").keycap("Shift+F11").compact())
                    .clicked()
                {
                    run = Some("debug.step_out");
                }
            } else if ui.add(Button::ghost("Pause").compact()).clicked() {
                state.debug_act(|d| d.pause());
            }
            if let Some(id) = run {
                state.request(crate::editor::UiRequest::RunAction(id));
            }
            ui.add_space(8.0);
            let mut on = state.debug.break_on_error;
            if ui.checkbox(&mut on, "Break on errors").changed() {
                state.set_break_on_error(on);
            }
        });
        ui.separator();
        let stack = match &self.state {
            Some(DebugState::Paused { stack, .. }) => stack.clone(),
            _ => Vec::new(),
        };
        ui.columns(3, |cols| {
            if let Some(frame) = call_stack::ui(&mut cols[0], &stack, self.frame) {
                self.frame = frame;
                self.values.clear();
                if let Some(script) = stack[frame].script {
                    self.reveal = Some((script, stack[frame].line));
                }
            }
            variables::ui(&mut cols[1], self, state, paused);
            watch::ui(&mut cols[2], self, state, paused);
        });
    }
}

fn reason_text(reason: &PauseReason) -> String {
    match reason {
        PauseReason::Breakpoint => "breakpoint".into(),
        PauseReason::Step => "step".into(),
        PauseReason::Pause => "paused".into(),
        PauseReason::Error(e) => format!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use metallic_core::{Value, World};
    use metallic_script::Breakpoint;

    use super::DebugPanel;
    use crate::editor::EditorState;
    use crate::output::OutputLog;

    const SOURCE: &str = "local hp = tonumber('100')\nhp = hp - 1\nprint('after', hp)\n";

    /// An edit session with one server script, and the script's id.
    fn session(log: &OutputLog) -> (EditorState, metallic_core::InstanceId) {
        let mut world = World::new_with_services();
        let parent = world.root("ServerScriptService").unwrap();
        let script = world.create("Script", Some(parent)).unwrap();
        world
            .set(script, "Source", Value::String(SOURCE.into()))
            .unwrap();
        let state = EditorState::new(
            world,
            Path::new("."),
            Path::new("main.mscene"),
            "Test",
            log.sink(),
        );
        (state, script)
    }

    fn texts(log: &mut OutputLog) -> Vec<String> {
        log.drain();
        log.lines().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn breakpoints_set_while_editing_pause_the_next_play() {
        let mut log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        state.toggle_breakpoint(script, 2);
        assert_eq!(state.debug.get(script, 2), Some(&Breakpoint::Plain));
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        assert!(
            state.is_paused(),
            "the script stops on line 2 before it runs"
        );

        let mut panel = DebugPanel::new();
        panel.poll(&mut state);
        assert_eq!(panel.paused_line(script), Some(2));
        assert_eq!(panel.take_reveal(), Some((script, 2)));
        assert_eq!(panel.hover_value(&mut state, "hp").as_deref(), Some("100"));
        assert!(
            texts(&mut log)
                .iter()
                .any(|t| t.starts_with("Paused: breakpoint"))
        );

        // Step over runs line 2 only; continue finishes the script.
        state.debug_act(|d| d.step_over());
        panel.poll(&mut state);
        assert_eq!(panel.paused_line(script), Some(3));
        assert_eq!(panel.hover_value(&mut state, "hp").as_deref(), Some("99"));
        state.debug_act(|d| d.resume());
        panel.poll(&mut state);
        assert!(!state.is_paused());
        assert!(
            texts(&mut log)
                .iter()
                .any(|t| t.contains("after\t99") || t.contains("after 99"))
        );
    }

    #[test]
    fn breakpoints_toggle_and_survive_stop() {
        let log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        state.toggle_breakpoint(script, 3);
        state.toggle_breakpoint(script, 3);
        assert!(state.debug.breakpoints.is_empty());
        state.set_breakpoint(script, 2, Some(Breakpoint::Condition("hp > 50".into())));
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        assert!(state.is_paused());
        state.stop_play();
        assert!(!state.is_paused());
        assert!(state.debugger().is_none());
        assert_eq!(
            state.debug.get(script, 2),
            Some(&Breakpoint::Condition("hp > 50".into())),
            "breakpoints belong to the edit session"
        );
    }

    #[test]
    fn stop_while_paused_then_play_again_runs_cleanly() {
        let mut log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        state.toggle_breakpoint(script, 3);
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        let mut panel = DebugPanel::new();
        panel.poll(&mut state);
        assert!(panel.is_paused());
        state.stop_play();
        panel.poll(&mut state);
        assert!(!panel.is_paused(), "Stop clears the paused view");
        assert_eq!(panel.paused_line(script), None);

        state.toggle_breakpoint(script, 3);
        state.set_break_on_error(true);
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        assert!(!state.is_paused());
        assert!(texts(&mut log).iter().any(|t| t.contains("99")));
        state.stop_play();
    }

    #[test]
    fn pause_then_step_into_stops_in_the_next_handler() {
        let log = OutputLog::new(64);
        let mut world = World::new_with_services();
        let parent = world.root("ServerScriptService").unwrap();
        let script = world.create("Script", Some(parent)).unwrap();
        let src = "local n = 0\ngame:GetService('RunService').Heartbeat:Connect(function()\n  n = n + 1\nend)\n";
        world
            .set(script, "Source", Value::String(src.into()))
            .unwrap();
        let mut state = EditorState::new(world, Path::new("."), Path::new("m"), "T", log.sink());
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        state.debug_act(|d| d.pause());
        let mut panel = DebugPanel::new();
        panel.poll(&mut state);
        assert!(panel.is_paused(), "Pause holds the game between frames");
        // Step Into from there stops in the next script code that runs (the handler).
        state.debug_act(|d| d.step_into());
        state.play.update(1.0 / 60.0);
        panel.poll(&mut state);
        assert_eq!(panel.paused_line(script), Some(3));
        state.stop_play();
    }
}
