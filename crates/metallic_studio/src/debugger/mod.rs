//! The script debugger in Studio (#22): breakpoints in the script editor's gutter, pause and
//! step during Play, and the Call Stack, Variables and Watch panes.
//!
//! Breakpoints live in [`DebugSettings`] on the [`EditorState`], so they survive Stop and apply
//! from the first line of the next Play ([`crate::play::PlayStart::debug`]). While a session runs
//! they are also sent to its game through [`crate::play::PlayView::debugger`]. The game runs on
//! the UI thread and a paused script *breaks* instead of blocking, so Studio stays usable while
//! paused: the viewport, Explorer and Properties keep working.
//!
//! Breakpoints stay on their lines when lines are added or removed above them
//! ([`EditorState::shift_breakpoints`]) and are saved per place in Studio's settings
//! ([`store::BreakpointStore`]), so they come back the next time the game is opened.
//!
//! Keys: F5 continues when paused (it is Play otherwise), F9 toggles a breakpoint on the script
//! editor's cursor line (during Play only while a script editor has focus, so games keep F9),
//! F10 steps over, F11 into, Shift+F11 out.

mod call_stack;
mod gutter;
mod store;
mod variables;
mod watch;

use std::collections::HashMap;
use std::rc::Rc;

use eframe::egui::{self, RichText};
use metallic_core::InstanceId;
use metallic_script::{
    Breakpoint, DebugState, Debugger, Frame, PauseReason, Scope, VarHandle, VarNode,
};

use crate::actions::{Action, ActionRegistry, ToolbarGroup};
use crate::editor::EditorState;
use crate::icons::Icon;
use crate::tokens::ui_tokens;
use crate::widgets::button::Button;

pub use gutter::{Gutter, LineRow, gutter, hover, line_rows, name_at};
pub use store::{BreakpointStore, Breakpoints, shift_lines};

/// Breakpoints and options of the script debugger, kept with the edit session.
#[derive(Debug, Default, Clone)]
pub struct DebugSettings {
    /// Breakpoints by script and line.
    pub breakpoints: Breakpoints,
    /// Pause at errors that no `pcall` handles.
    pub break_on_error: bool,
    /// The script and line F9 toggles: the shown script editor's cursor line while editing, and
    /// during Play only while a script editor has focus. Set by the script editor each frame.
    pub cursor: Option<(InstanceId, u32)>,
    /// Bumped by every debugger action, so cached views refresh.
    generation: u64,
    /// Bumped by Continue, the steps and Pause: a pause seen after it is a new pause.
    runs: u64,
    /// Where breakpoints are saved (`None`: not saved, as in tests and screenshots).
    store: Option<BreakpointStore>,
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

    /// Loads this place's saved breakpoints from `store` and saves later changes there.
    pub fn load(&mut self, store: BreakpointStore, world: &metallic_core::World) {
        let (breakpoints, break_on_error) = store.load(world);
        self.breakpoints = breakpoints;
        self.break_on_error = break_on_error;
        self.store = Some(store);
        self.generation += 1;
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

    /// Writes the breakpoints to this place's entry in Studio's settings, when it has one.
    fn save_breakpoints(&self) {
        if !self.can_save {
            return;
        }
        if let Some(store) = &self.debug.store
            && let Err(e) = store.save(
                &self.world,
                &self.debug.breakpoints,
                self.debug.break_on_error,
            )
        {
            log::warn!("could not save breakpoints: {e}");
        }
    }

    /// Moves `script`'s breakpoints with its lines after an edit from `old` to `new` text.
    pub fn shift_breakpoints(&mut self, script: InstanceId, old: &str, new: &str) {
        let Some(lines) = self.debug.breakpoints.get(&script) else {
            return;
        };
        let moved = shift_lines(lines, old, new);
        if &moved == lines {
            return;
        }
        if moved.is_empty() {
            self.debug.breakpoints.remove(&script);
        } else {
            self.debug.breakpoints.insert(script, moved);
        }
        self.debug.generation += 1;
        self.save_breakpoints();
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
        self.save_breakpoints();
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
        self.save_breakpoints();
    }

    /// Runs `act` (continue, a step, pause) on the game's debugger and marks views stale.
    pub fn debug_act(&mut self, act: impl FnOnce(&mut Debugger<'_>)) {
        if let Some(mut d) = self.debugger() {
            act(&mut d);
        }
        self.debug.generation += 1;
        self.debug.runs += 1;
    }
}

/// A debugger command run by an action.
type DebugAct = fn(&mut Debugger<'_>);

/// Registers the debugger's actions (`debug.*`).
pub fn register(r: &mut ActionRegistry) {
    // On the script editor's cursor line ([`DebugSettings::cursor`]); disabled otherwise, which
    // leaves F9 to a running game.
    r.register(
        Action::new(
            "debug.toggle_breakpoint",
            "Toggle Breakpoint",
            Icon::Text(""),
            ToolbarGroup::Hidden,
            |s| {
                if let Some((script, line)) = s.debug.cursor {
                    s.toggle_breakpoint(script, line);
                }
            },
        )
        .enabled(|s| s.debug.cursor.is_some()),
    );
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
    /// The pause when last read (`None` while running).
    paused: Option<Paused>,
    seen: Option<u64>,
    was_paused: bool,
    /// The [`DebugSettings::runs`] of the pause last announced in Output.
    announced: Option<u64>,
    /// Caches for the selected frame, cleared when the game runs or the frame changes; shared so
    /// drawing them each frame copies nothing.
    vars: HashMap<u8, Rc<[VarNode]>>,
    expanded: HashMap<VarHandle, Rc<[VarNode]>>,
    values: HashMap<String, Result<Rc<str>, Rc<str>>>,
    editing: Option<Editing>,
    setting: Option<SetLocal>,
    /// A script to show at a line (the paused frame), taken by the editor UI.
    reveal: Option<(InstanceId, u32)>,
}

/// A paused game as last read.
struct Paused {
    reason: PauseReason,
    stack: Rc<[Frame]>,
}

impl DebugPanel {
    /// A pane with no watches.
    pub fn new() -> DebugPanel {
        DebugPanel::default()
    }

    /// Reads the game's debugger state when it changed. Once per pause (the game stopping, or
    /// stopping again after Continue or a step): a line in Output and a request to show the
    /// paused script ([`DebugPanel::take_reveal`]). Breakpoint edits and Set value while paused
    /// only refresh the panes.
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
        let was = self.paused.is_some();
        self.paused = match state.debugger().map(|d| d.state()) {
            Some(DebugState::Paused { reason, stack }) => Some(Paused {
                reason,
                stack: stack.into(),
            }),
            _ => None,
        };
        let Some(p) = &self.paused else {
            self.frame = 0;
            return;
        };
        let runs = state.debug.runs;
        if was && self.announced == Some(runs) {
            if self.frame >= p.stack.len() {
                self.frame = 0;
            }
            return;
        }
        self.announced = Some(runs);
        self.frame = 0;
        match p.stack.first() {
            Some(top) => {
                if let Some(script) = top.script {
                    self.reveal = Some((script, top.line));
                }
                state.output.studio(format!(
                    "Paused: {} at {}:{}",
                    reason_text(&p.reason),
                    top.chunk,
                    top.line
                ));
            }
            None => state.output.studio("Paused"),
        }
    }

    /// The script and line to show after a pause or a frame click, once.
    pub fn take_reveal(&mut self) -> Option<(InstanceId, u32)> {
        self.reveal.take()
    }

    /// The paused line of `script` in the selected frame, for the editor's marker.
    pub fn paused_line(&self, script: InstanceId) -> Option<u32> {
        self.paused
            .as_ref()?
            .stack
            .get(self.frame)
            .filter(|f| f.script == Some(script))
            .map(|f| f.line)
    }

    /// True while paused (as of the last [`DebugPanel::poll`]).
    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    /// The value of `expr` in the selected frame, for hover tips over `script` (cached until
    /// the game runs). `None` unless `script` is the selected frame's script: a name in another
    /// script would be looked up in the wrong frame.
    pub fn hover_value(
        &mut self,
        state: &mut EditorState,
        script: InstanceId,
        expr: &str,
    ) -> Option<Rc<str>> {
        self.paused_line(script)?;
        self.value(state, expr).ok()
    }

    /// `expr` evaluated in the selected frame (cached; the cache is cleared with the frame).
    fn value(&mut self, state: &mut EditorState, expr: &str) -> Result<Rc<str>, Rc<str>> {
        if let Some(v) = self.values.get(expr) {
            return v.clone();
        }
        let v = match state.debugger() {
            Some(d) => d
                .evaluate(self.frame, expr)
                .map(Rc::from)
                .map_err(|e| Rc::from(e.message)),
            None => Err(Rc::from("not playing")),
        };
        self.values.insert(expr.to_string(), v.clone());
        v
    }

    fn scope(&mut self, state: &mut EditorState, scope: Scope, key: u8) -> Rc<[VarNode]> {
        if let Some(v) = self.vars.get(&key) {
            return v.clone();
        }
        let v: Rc<[VarNode]> = state
            .debugger()
            .map(|d| d.variables(self.frame, scope))
            .unwrap_or_default()
            .into();
        self.vars.insert(key, v.clone());
        v
    }

    /// Selects frame `frame` of the stack: Variables, Watch and hover values follow it.
    fn select_frame(&mut self, frame: usize) {
        self.frame = frame;
        self.vars.clear();
        self.expanded.clear();
        self.values.clear();
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

    fn expand(&mut self, state: &mut EditorState, handle: VarHandle) -> Rc<[VarNode]> {
        if let Some(v) = self.expanded.get(&handle) {
            return v.clone();
        }
        let v: Rc<[VarNode]> = state
            .debugger()
            .map(|d| d.expand(handle))
            .unwrap_or_default()
            .into();
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
            let status = match &self.paused {
                Some(p) => match p.stack.first() {
                    Some(f) => {
                        format!(
                            "Paused ({}) at {}:{}",
                            reason_text(&p.reason),
                            f.chunk,
                            f.line
                        )
                    }
                    None => "Paused".to_string(),
                },
                None => "Running".to_string(),
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
        let stack: Rc<[Frame]> = match &self.paused {
            Some(p) => p.stack.clone(),
            None => Rc::from([]),
        };
        ui.columns(3, |cols| {
            if let Some(frame) = call_stack::ui(&mut cols[0], &stack, self.frame) {
                self.select_frame(frame);
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
        assert_eq!(
            panel.hover_value(&mut state, script, "hp").as_deref(),
            Some("100")
        );
        assert!(
            texts(&mut log)
                .iter()
                .any(|t| t.starts_with("Paused: breakpoint"))
        );

        // Step over runs line 2 only; continue finishes the script.
        state.debug_act(|d| d.step_over());
        panel.poll(&mut state);
        assert_eq!(panel.paused_line(script), Some(3));
        assert_eq!(
            panel.hover_value(&mut state, script, "hp").as_deref(),
            Some("99")
        );
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

    #[test]
    fn a_pause_is_announced_once() {
        let mut log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        state.toggle_breakpoint(script, 2);
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        let mut panel = DebugPanel::new();
        panel.poll(&mut state);
        assert_eq!(panel.take_reveal(), Some((script, 2)));
        let paused = |log: &mut OutputLog| {
            texts(log)
                .iter()
                .filter(|t| t.starts_with("Paused"))
                .count()
        };
        assert_eq!(paused(&mut log), 1);

        // Breakpoint edits and Set value while paused refresh the panes only.
        state.toggle_breakpoint(script, 3);
        panel.poll(&mut state);
        state.toggle_breakpoint(script, 3);
        panel.poll(&mut state);
        assert_eq!(panel.take_reveal(), None, "the editor is not pulled back");
        assert_eq!(paused(&mut log), 1);

        // A step is a new pause: one more line and one reveal.
        state.debug_act(|d| d.step_over());
        panel.poll(&mut state);
        panel.poll(&mut state);
        assert_eq!(panel.take_reveal(), Some((script, 3)));
        assert_eq!(paused(&mut log), 2);
        state.stop_play();
    }

    #[test]
    fn hover_values_only_over_the_paused_frames_script() {
        let log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        let other = state
            .world
            .create("Script", state.world.parent(script))
            .unwrap();
        state.toggle_breakpoint(script, 3);
        assert!(state.start_play("play.start"));
        state.play.update(1.0 / 60.0);
        let mut panel = DebugPanel::new();
        panel.poll(&mut state);
        assert_eq!(
            panel.hover_value(&mut state, script, "hp").as_deref(),
            Some("99")
        );
        assert_eq!(
            panel.hover_value(&mut state, other, "hp"),
            None,
            "a name in another script is not this frame's"
        );
        state.stop_play();
    }

    #[test]
    fn breakpoints_follow_edits_and_f9_needs_a_cursor() {
        let log = OutputLog::new(64);
        let (mut state, script) = session(&log);
        state.toggle_breakpoint(script, 2);
        state.toggle_breakpoint(script, 3);
        let edited = format!("-- new first line\n{SOURCE}");
        state.shift_breakpoints(script, SOURCE, &edited);
        assert_eq!(
            state.debug.breakpoints[&script]
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [3, 4]
        );

        let mut actions = crate::actions::ActionRegistry::new();
        super::register(&mut actions);
        let f9 = actions.get("debug.toggle_breakpoint").unwrap();
        assert!(
            !f9.is_enabled(&state),
            "no script editor cursor: F9 is free"
        );
        state.debug.cursor = Some((script, 1));
        assert!(f9.is_enabled(&state));
        actions.run("debug.toggle_breakpoint", &mut state);
        assert!(state.debug.get(script, 1).is_some());
    }
}
