//! The input action runtime: raw events in, action values and events out.
//!
//! One [`InputRuntime`] belongs to one game on the client. Per frame: [`push`](InputRuntime::push)
//! raw events (or [`poll_source`](InputRuntime::poll_source)), call
//! [`evaluate`](InputRuntime::evaluate) once, then read [`state`](InputRuntime::state) and
//! [`drain_events`](InputRuntime::drain_events). Evaluation allocates nothing: the bindings are
//! compiled once into dense arrays (a list of binding indices per control) by
//! [`load`](InputRuntime::load).
//!
//! Order of work for one binding (spec 3.5): modifiers, dead zone, response curve, scale, delta
//! time, clamp. Per action: Bool is the OR of its bindings, vectors take the binding with the
//! largest magnitude. Bool actions are evaluated at the time of each raw event, so a tap that
//! starts and ends inside one frame still fires, and holds and taps are measured in event time.
use super::FrameInput;
use super::config::{ActionType, BindingDef, CHARACTER_CONTROLS, InputConfig, Role, Trigger};
use super::control::{
    CONTROL_COUNT, Control, ControlKind, DEVICE_SLOTS, Device, DeviceKind, InputQueue,
    QUEUE_CAPACITY, RawEvent, RawSource,
};
use metallic_core::{ChangeCursor, Changes, InstanceId, World};
use std::collections::{HashSet, VecDeque};
use std::ops::Range;

/// Events kept for [`InputRuntime::drain_events`]; when full the oldest is dropped.
const EVENT_CAPACITY: usize = 1024;
/// Slack when comparing event times (seconds).
const EPS: f64 = 1e-9;
/// Most repeats one `evaluate` fires for one action (a long pause must not burst).
const MAX_REPEATS_PER_CALL: u32 = 32;

/// Mouse movement that counts as a meaningful use of the mouse, in pixels ...
const MOUSE_MOVE_PIXELS: f32 = 12.0;
/// ... within this many seconds (spec 4.4).
const MOUSE_MOVE_WINDOW: f64 = 0.2;
/// A stick or trigger beyond this counts as a gamepad press (spec 4.4).
const PAD_ACTIVE: f32 = 0.5;
/// The current device must have been quiet this long before another one takes over.
const SWITCH_QUIET: f64 = 0.3;
/// Two switches are at least this far apart.
const SWITCH_GAP: f64 = 0.5;

/// Handle of an action inside one [`InputRuntime`]; an index, so lookups in the hot path are free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(pub u16);

/// What an action currently holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionValue {
    /// A Bool action.
    Bool(bool),
    /// A Direction1D action.
    Axis1(f32),
    /// A Direction2D or ViewportPosition action.
    Axis2([f32; 2]),
    /// A Direction3D action.
    Axis3([f32; 3]),
}

/// What happened to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEventKind {
    /// The raw press edge of a Bool action.
    Pressed,
    /// The raw release edge of a Bool action.
    Released,
    /// The action's trigger fired (for vector actions: the value changed).
    Triggered,
    /// The value changed since the last frame.
    StateChanged,
}

/// One event of one action, for scripts to connect to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionEvent {
    /// The action.
    pub action: ActionId,
    /// What happened.
    pub kind: ActionEventKind,
    /// The action's value right after.
    pub value: ActionValue,
    /// Event time in seconds.
    pub time: f64,
}

struct CompiledAction {
    path: String,
    ty: ActionType,
    role: Role,
    trigger: Trigger,
    active: bool,
    bindings: Range<usize>,
    instance: Option<InstanceId>,
    /// A script handler answered `Sink` for this action's press.
    script_sink: bool,
    hold: f64,
    tap: f64,
    double_tap: f64,
    repeat_delay: f64,
    repeat_rate: f64,
    // State.
    value: [f32; 3],
    emitted: [f32; 3],
    down: bool,
    down_since: f64,
    hold_fired: bool,
    last_tap_release: f64,
    next_repeat: f64,
}

struct CompiledBinding {
    action: usize,
    /// Rank of the owning context: 0 is asked first.
    rank: u16,
    def: BindingDef,
    deadzone: f32,
    /// Analog hysteresis state for Bool actions.
    pressed: bool,
}

/// Evaluates input actions. See the module docs.
pub struct InputRuntime {
    actions: Vec<CompiledAction>,
    bindings: Vec<CompiledBinding>,
    /// Per control: the bindings that read it.
    by_control: Vec<Vec<usize>>,
    /// Per control: the best rank of a sinking context that uses it (`u16::MAX`: none).
    sink_rank: [u16; CONTROL_COUNT],
    /// Like `sink_rank`, for actions whose script handler answered `Sink` (spec 3.7).
    script_sink_rank: [u16; CONTROL_COUNT],
    /// True while some action has `script_sink` set.
    any_script_sink: bool,
    /// Per device slot and control: the last value; the vector for two-axis controls.
    slot_value: [[f32; CONTROL_COUNT]; DEVICE_SLOTS],
    slot_pos: [[[f32; 2]; CONTROL_COUNT]; DEVICE_SLOTS],
    /// Per control: all devices combined.
    value: [f32; CONTROL_COUNT],
    pos: [[f32; 2]; CONTROL_COUNT],
    delta_touched: Vec<Control>,
    delta_flag: [bool; CONTROL_COUNT],
    queue: InputQueue,
    scratch: Vec<RawEvent>,
    events: VecDeque<ActionEvent>,
    dropped_events: u64,
    // Triggers that need the clock.
    timed: Vec<usize>,
    timers_active: u32,
    clock: f64,
    dt: f32,
    // GUI.
    gui_pointer: bool,
    gui_text: bool,
    dirty_all: bool,
    // Last-used device.
    last_device: DeviceKind,
    last_switch: f64,
    last_meaningful: [f64; 3],
    held: [u32; 3],
    mouse_acc: f32,
    mouse_since: f64,
    last_pointer: Option<[f32; 2]>,
    device_changes: u64,
    // Scene link.
    cursor: Option<ChangeCursor>,
    watched: HashSet<InstanceId>,
    context_orders: Vec<(String, bool, u64)>,
    order_counter: u64,
    warnings: Vec<String>,
    character: [Option<ActionId>; 5],
}

impl Default for InputRuntime {
    fn default() -> Self {
        Self::new(&InputConfig::character_controls())
    }
}

impl std::fmt::Debug for InputRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputRuntime")
            .field("actions", &self.actions.len())
            .field("bindings", &self.bindings.len())
            .field("last_device", &self.last_device)
            .finish()
    }
}

fn len2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn len3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl InputRuntime {
    /// A runtime for `config`.
    pub fn new(config: &InputConfig) -> Self {
        let mut rt = InputRuntime {
            actions: Vec::new(),
            bindings: Vec::new(),
            by_control: vec![Vec::new(); CONTROL_COUNT],
            sink_rank: [u16::MAX; CONTROL_COUNT],
            script_sink_rank: [u16::MAX; CONTROL_COUNT],
            any_script_sink: false,
            slot_value: [[0.0; CONTROL_COUNT]; DEVICE_SLOTS],
            slot_pos: [[[0.0; 2]; CONTROL_COUNT]; DEVICE_SLOTS],
            value: [0.0; CONTROL_COUNT],
            pos: [[0.0; 2]; CONTROL_COUNT],
            delta_touched: Vec::with_capacity(CONTROL_COUNT),
            delta_flag: [false; CONTROL_COUNT],
            queue: InputQueue::new(),
            scratch: Vec::with_capacity(QUEUE_CAPACITY),
            events: VecDeque::with_capacity(EVENT_CAPACITY),
            dropped_events: 0,
            timed: Vec::new(),
            timers_active: 0,
            clock: 0.0,
            dt: 0.0,
            gui_pointer: false,
            gui_text: false,
            dirty_all: false,
            last_device: DeviceKind::KeyboardMouse,
            last_switch: f64::NEG_INFINITY,
            last_meaningful: [f64::NEG_INFINITY; 3],
            held: [0; 3],
            mouse_acc: 0.0,
            mouse_since: f64::NEG_INFINITY,
            last_pointer: None,
            device_changes: 0,
            cursor: None,
            watched: HashSet::new(),
            context_orders: Vec::new(),
            order_counter: 1 << 32,
            warnings: Vec::new(),
            character: [None; 5],
        };
        rt.load(config);
        rt
    }

    /// A runtime for the contexts in `world` (the built-in `CharacterControls` when the scene has
    /// none); [`sync`](Self::sync) keeps it up to date.
    pub fn from_world(world: &World) -> Self {
        let mut rt = Self::new(&InputConfig::from_world(world));
        rt.cursor = Some(world.change_cursor());
        rt
    }

    /// Replaces the contexts. Raw control state and the state of actions that exist both before
    /// and after (matched by path) are kept, so a held key does not fire a second press.
    pub fn load(&mut self, config: &InputConfig) {
        let old: Vec<(String, bool, f64, bool, f64)> = self
            .actions
            .iter()
            .map(|a| {
                (
                    a.path.clone(),
                    a.down,
                    a.down_since,
                    a.hold_fired,
                    a.last_tap_release,
                )
            })
            .collect();
        self.actions.clear();
        self.bindings.clear();
        self.timed.clear();
        self.timers_active = 0;
        for list in &mut self.by_control {
            list.clear();
        }
        self.sink_rank = [u16::MAX; CONTROL_COUNT];
        self.script_sink_rank = [u16::MAX; CONTROL_COUNT];
        self.any_script_sink = false;
        self.warnings = config.warnings.clone();

        // Context order: priority, then the context enabled most recently.
        let mut orders = Vec::with_capacity(config.contexts.len());
        for ctx in &config.contexts {
            let before = self.context_orders.iter().find(|(n, _, _)| *n == ctx.name);
            let order = match before {
                Some((_, was_enabled, order)) if *was_enabled || !ctx.enabled => *order,
                Some(_) => {
                    self.order_counter += 1;
                    self.order_counter
                }
                None => ctx.order,
            };
            orders.push(order);
        }
        self.context_orders = config
            .contexts
            .iter()
            .zip(&orders)
            .map(|(c, &o)| (c.name.clone(), c.enabled, o))
            .collect();
        let mut ranked: Vec<usize> = (0..config.contexts.len())
            .filter(|&i| config.contexts[i].enabled)
            .collect();
        ranked.sort_by(|&a, &b| {
            config.contexts[b]
                .priority
                .total_cmp(&config.contexts[a].priority)
                .then(orders[b].cmp(&orders[a]))
        });
        let mut rank_of = vec![u16::MAX; config.contexts.len()];
        for (rank, &i) in ranked.iter().enumerate() {
            rank_of[i] = rank.min(u16::MAX as usize - 1) as u16;
        }

        for (ci, ctx) in config.contexts.iter().enumerate() {
            for a in &ctx.actions {
                let active = ctx.enabled && a.enabled;
                let start = self.bindings.len();
                let action_index = self.actions.len();
                if active {
                    for b in &a.bindings {
                        let deadzone = b
                            .deadzone
                            .or_else(|| b.key.map(Control::default_deadzone))
                            .unwrap_or(0.0);
                        let index = self.bindings.len();
                        for control in b.controls() {
                            let list = &mut self.by_control[control.index()];
                            if list.last() != Some(&index) {
                                list.push(index);
                            }
                            if ctx.sink {
                                let r = &mut self.sink_rank[control.index()];
                                *r = (*r).min(rank_of[ci]);
                            }
                        }
                        self.bindings.push(CompiledBinding {
                            action: action_index,
                            rank: rank_of[ci],
                            def: b.clone(),
                            deadzone,
                            pressed: false,
                        });
                    }
                }
                let path = format!("{}/{}", ctx.name, a.name);
                let before = old.iter().find(|o| o.0 == path);
                let ms = |v: f32| f64::from(v) / 1000.0;
                self.actions.push(CompiledAction {
                    path,
                    ty: a.ty,
                    role: a.role,
                    trigger: a.trigger,
                    active,
                    bindings: start..self.bindings.len(),
                    instance: a.instance,
                    script_sink: false,
                    hold: ms(a.hold_time),
                    tap: ms(a.tap_time),
                    double_tap: ms(a.double_tap_window),
                    repeat_delay: ms(a.repeat_delay),
                    repeat_rate: ms(a.repeat_rate).max(0.001),
                    value: [0.0; 3],
                    emitted: [0.0; 3],
                    down: before.is_some_and(|o| o.1) && active,
                    down_since: before.map_or(0.0, |o| o.2),
                    hold_fired: before.is_some_and(|o| o.3),
                    last_tap_release: before.map_or(f64::NEG_INFINITY, |o| o.4),
                    next_repeat: f64::INFINITY,
                });
                let last = self.actions.len() - 1;
                if active && matches!(a.trigger, Trigger::Hold | Trigger::Repeat) {
                    self.timed.push(last);
                    if self.actions[last].down {
                        self.timers_active += 1;
                        self.actions[last].next_repeat =
                            self.clock + self.actions[last].repeat_delay;
                    }
                }
            }
        }
        self.watched = config.instances().into_iter().collect();
        self.character = [
            self.find_action(&format!("{CHARACTER_CONTROLS}/Move")),
            self.find_action(&format!("{CHARACTER_CONTROLS}/Jump")),
            self.find_action(&format!("{CHARACTER_CONTROLS}/Run")),
            self.find_action(&format!("{CHARACTER_CONTROLS}/Look")),
            self.find_action(&format!("{CHARACTER_CONTROLS}/Zoom")),
        ];
        self.dirty_all = true;
    }

    /// Reloads the contexts when the world's change journal reports that an input instance (or
    /// anything of unknown class, such as a destroyed one) changed. Returns true when it did.
    pub fn sync(&mut self, world: &World) -> bool {
        let Some(cursor) = self.cursor else {
            self.cursor = Some(world.change_cursor());
            return false;
        };
        let (changes, now) = world.changes_since(cursor);
        self.cursor = Some(now);
        let changed = match changes {
            Changes::Overflow => true,
            Changes::Ids(ids) => ids.iter().any(|&id| {
                self.watched.contains(&id)
                    || matches!(
                        world.class_name(id),
                        Some("InputContext" | "InputAction" | "InputBinding")
                    )
            }),
        };
        if changed {
            let config = InputConfig::from_world(world);
            self.load(&config);
        }
        changed
    }

    /// Problems found while reading the scene.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The action with this `Context/Action` path.
    pub fn find_action(&self, path: &str) -> Option<ActionId> {
        self.actions
            .iter()
            .position(|a| a.path == path)
            .map(|i| ActionId(i as u16))
    }

    /// The action made from this `InputAction` instance.
    pub fn action_for_instance(&self, id: InstanceId) -> Option<ActionId> {
        self.actions
            .iter()
            .position(|a| a.instance == Some(id))
            .map(|i| ActionId(i as u16))
    }

    /// The `InputAction` instance an action was made from (none for the built-in context).
    pub fn instance(&self, action: ActionId) -> Option<InstanceId> {
        self.actions.get(action.0 as usize).and_then(|a| a.instance)
    }

    /// Records the answer of a script handler (`ContextActionService`, spec 3.7) for `action`:
    /// with `sink` the controls of its bindings read 0 for lower contexts for as long as the
    /// action stays down; `Pass` (false) lets them through. Takes effect at once for the
    /// current frame: lower actions are evaluated again (so a jump already seen is released
    /// before the character reads it). A sink on an action that is already up has no effect.
    pub fn set_script_sink(&mut self, action: ActionId, sink: bool, now: f64) {
        let Some(a) = self.actions.get_mut(action.0 as usize) else {
            return;
        };
        if a.script_sink == sink {
            return;
        }
        a.script_sink = sink;
        self.any_script_sink = self.actions.iter().any(|a| a.script_sink);
        if self.rebuild_script_sinks() {
            self.reevaluate(now);
        }
    }

    /// Recomputes `script_sink_rank`; actions that went up lose their sink. True when it changed.
    fn rebuild_script_sinks(&mut self) -> bool {
        let mut next = [u16::MAX; CONTROL_COUNT];
        let mut any = false;
        for a in 0..self.actions.len() {
            if !self.actions[a].script_sink {
                continue;
            }
            let act = &self.actions[a];
            let held = act.active
                && if act.ty == ActionType::Bool {
                    act.down
                } else {
                    act.value != [0.0; 3]
                };
            if !held {
                self.actions[a].script_sink = false;
                continue;
            }
            any = true;
            for bi in self.actions[a].bindings.clone() {
                let b = &self.bindings[bi];
                for c in b.def.controls() {
                    let r = &mut next[c.index()];
                    *r = (*r).min(b.rank);
                }
            }
        }
        self.any_script_sink = any;
        let changed = next != self.script_sink_rank;
        self.script_sink_rank = next;
        changed
    }

    /// Evaluates the actions again without new input, after sinks changed. Mouse movement and
    /// wheel notches of this frame are already used up, so actions that read them stay as they are.
    fn reevaluate(&mut self, now: f64) {
        for a in 0..self.actions.len() {
            if !self.actions[a].active {
                continue;
            }
            if self.actions[a].ty == ActionType::Bool {
                self.recompute_bool(a, now);
                continue;
            }
            let per_frame = self.actions[a].bindings.clone().any(|bi| {
                self.bindings[bi]
                    .def
                    .controls()
                    .any(|c| matches!(c.kind(), ControlKind::Delta | ControlKind::Wheel))
            });
            if !per_frame {
                self.recompute_vector(a);
                self.announce_vector(a, now);
            }
        }
    }

    /// The path `Context/Action` of an action.
    pub fn path(&self, action: ActionId) -> &str {
        self.actions
            .get(action.0 as usize)
            .map_or("", |a| a.path.as_str())
    }

    /// The role of an action.
    pub fn role(&self, action: ActionId) -> Role {
        self.actions
            .get(action.0 as usize)
            .map_or(Role::None, |a| a.role)
    }

    /// Number of actions (enabled or not).
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    // -----------------------------------------------------------------------------------------
    // Input

    /// Queues a raw event for the next [`evaluate`](Self::evaluate). Events with a value that is
    /// not a finite number are dropped.
    pub fn push(&mut self, e: RawEvent) {
        let finite = e.value.is_finite()
            && e.time.is_finite()
            && e.pos.is_none_or(|p| p[0].is_finite() && p[1].is_finite());
        if finite {
            self.queue.push(e);
        }
    }

    /// Asks `source` for its events and queues them.
    pub fn poll_source(&mut self, source: &mut dyn RawSource) {
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        source.poll(&mut scratch);
        for e in scratch.drain(..) {
            self.push(e);
        }
        self.scratch = scratch;
    }

    /// Tells the runtime that the GUI holds the pointer (a button is pressed) and/or the keyboard
    /// (a `TextBox` has focus). Controls the GUI took read as 0 for actions (spec 3.7).
    pub fn set_gui_capture(&mut self, pointer: bool, text_focus: bool) {
        if (pointer, text_focus) != (self.gui_pointer, self.gui_text) {
            self.gui_pointer = pointer;
            self.gui_text = text_focus;
            self.dirty_all = true;
        }
    }

    /// Releases every control, for example when the window loses focus. Held actions get their
    /// release edge at the next `evaluate`.
    pub fn release_all(&mut self, time: f64) {
        for slot in 0..DEVICE_SLOTS {
            for c in Control::ALL {
                let i = c.index();
                let idle = self.slot_value[slot][i] == 0.0 && self.slot_pos[slot][i] == [0.0; 2];
                if idle {
                    continue;
                }
                let device = match slot {
                    0 => Device::KeyboardMouse,
                    s if s <= super::MAX_GAMEPADS => Device::Gamepad((s - 1) as u8),
                    _ => Device::Touch,
                };
                let e = match c.kind() {
                    ControlKind::Button | ControlKind::Wheel => {
                        RawEvent::button(device, *c, 0.0, time)
                    }
                    _ => RawEvent::vector(device, *c, [0.0; 2], time),
                };
                self.queue.push(e);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Evaluation

    /// Evaluates every action once for a frame that ends at `now` (monotonic seconds) and lasted
    /// `dt` seconds. Allocates nothing.
    pub fn evaluate(&mut self, now: f64, dt: f32) {
        let now = if now.is_finite() {
            now.max(self.clock)
        } else {
            self.clock
        };
        self.dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if self.dirty_all {
            self.dirty_all = false;
            for a in 0..self.actions.len() {
                if self.actions[a].ty == ActionType::Bool {
                    self.recompute_bool(a, now);
                }
            }
        }
        while let Some(e) = self.queue.pop() {
            self.apply(e);
        }
        self.advance_timers(now);
        for a in 0..self.actions.len() {
            if self.actions[a].active && self.actions[a].ty != ActionType::Bool {
                self.recompute_vector(a);
                self.announce_vector(a, now);
            }
        }
        // Movement and wheel notches belong to one frame.
        for i in 0..self.delta_touched.len() {
            let c = self.delta_touched[i];
            self.delta_flag[c.index()] = false;
            self.value[c.index()] = 0.0;
            self.pos[c.index()] = [0.0; 2];
            for j in 0..self.by_control[c.index()].len() {
                let b = self.by_control[c.index()][j];
                let a = self.bindings[b].action;
                if self.actions[a].ty == ActionType::Bool {
                    self.recompute_bool(a, now);
                }
            }
        }
        self.delta_touched.clear();
        self.clock = now.max(self.clock);
        // A sunk action that went up no longer blocks the controls below it.
        if self.any_script_sink && self.rebuild_script_sinks() {
            self.reevaluate(now);
        }
    }

    /// Evaluates and returns the character's input (spec 6.2).
    pub fn frame(&mut self, now: f64, dt: f32) -> FrameInput {
        self.evaluate(now, dt);
        self.frame_input()
    }

    /// The `CharacterControls` actions as a [`FrameInput`]; all zero when the scene has no such
    /// context, or the action is missing or disabled.
    pub fn frame_input(&self) -> FrameInput {
        let [mv, jump, run, look, zoom] = self.character;
        let axis2 = |id: Option<ActionId>| id.map_or([0.0; 2], |id| self.axis2(id));
        let look = axis2(look);
        FrameInput {
            movement: axis2(mv),
            jump: jump.is_some_and(|id| self.bool(id)),
            run: run.is_some_and(|id| self.bool(id)),
            orbit: [look[0], -look[1]],
            zoom: zoom.map_or(0.0, |id| self.axis1(id)),
        }
    }

    /// Takes the events of the frames evaluated since the last call.
    pub fn drain_events(&mut self) -> std::collections::vec_deque::Drain<'_, ActionEvent> {
        self.events.drain(..)
    }

    /// Takes the oldest undrained event.
    pub fn pop_event(&mut self) -> Option<ActionEvent> {
        self.events.pop_front()
    }

    /// Events dropped because nobody drained them: the buffer keeps the newest
    /// [`EVENT_CAPACITY`] and never reallocates.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// The value of an action; zero for an unknown id.
    pub fn state(&self, action: ActionId) -> ActionValue {
        let Some(a) = self.actions.get(action.0 as usize) else {
            return ActionValue::Bool(false);
        };
        match a.ty {
            ActionType::Bool => ActionValue::Bool(a.down),
            ActionType::Direction1D => ActionValue::Axis1(a.value[0]),
            ActionType::Direction2D | ActionType::ViewportPosition => {
                ActionValue::Axis2([a.value[0], a.value[1]])
            }
            ActionType::Direction3D => ActionValue::Axis3(a.value),
        }
    }

    /// True while a Bool action is down (any other type: while its value is not zero).
    pub fn bool(&self, action: ActionId) -> bool {
        self.actions.get(action.0 as usize).is_some_and(|a| {
            if a.ty == ActionType::Bool {
                a.down
            } else {
                a.value != [0.0; 3]
            }
        })
    }

    /// The first axis of an action.
    pub fn axis1(&self, action: ActionId) -> f32 {
        self.actions
            .get(action.0 as usize)
            .map_or(0.0, |a| a.value[0])
    }

    /// The first two axes of an action.
    pub fn axis2(&self, action: ActionId) -> [f32; 2] {
        self.actions
            .get(action.0 as usize)
            .map_or([0.0; 2], |a| [a.value[0], a.value[1]])
    }

    /// All three axes of an action.
    pub fn axis3(&self, action: ActionId) -> [f32; 3] {
        self.actions
            .get(action.0 as usize)
            .map_or([0.0; 3], |a| a.value)
    }

    /// How far a Hold action is, 0 to 1 (for a progress ring); 0 for other triggers.
    pub fn hold_progress(&self, action: ActionId) -> f32 {
        let Some(a) = self.actions.get(action.0 as usize) else {
            return 0.0;
        };
        if a.trigger != Trigger::Hold || !a.down {
            return 0.0;
        }
        if a.hold_fired || a.hold <= 0.0 {
            return 1.0;
        }
        (((self.clock - a.down_since) / a.hold) as f32).clamp(0.0, 1.0)
    }

    /// True when an enabled binding on `MouseDelta` has its modifiers down: the Player should
    /// capture the cursor and send mouse movement (spec 4.1).
    pub fn wants_mouse_delta(&self) -> bool {
        self.by_control[Control::MouseDelta.index()]
            .iter()
            .any(|&b| {
                let bind = &self.bindings[b];
                bind.def.key == Some(Control::MouseDelta)
                    && bind
                        .def
                        .modifiers
                        .iter()
                        .flatten()
                        .all(|&m| self.read(m, bind.rank) >= 0.5)
            })
    }

    /// The device used last (spec 4.4).
    pub fn last_device(&self) -> DeviceKind {
        self.last_device
    }

    /// How often [`last_device`](Self::last_device) changed; compare with an earlier value to
    /// notice a change.
    pub fn device_changes(&self) -> u64 {
        self.device_changes
    }

    // -----------------------------------------------------------------------------------------
    // Internals

    /// The value of a control for a binding of context rank `rank`, after sinking and the GUI.
    fn read(&self, c: Control, rank: u16) -> f32 {
        if self.blocked(c, rank) {
            0.0
        } else {
            self.value[c.index()]
        }
    }

    fn read_pos(&self, c: Control, rank: u16) -> [f32; 2] {
        if self.blocked(c, rank) {
            [0.0; 2]
        } else {
            self.pos[c.index()]
        }
    }

    fn blocked(&self, c: Control, rank: u16) -> bool {
        self.sink_rank[c.index()] < rank
            || self.script_sink_rank[c.index()] < rank
            || if c.is_pointer() {
                self.gui_pointer
            } else {
                self.gui_text && !c.is_gamepad()
            }
    }

    fn apply(&mut self, e: RawEvent) {
        let t = e.time.max(self.clock);
        self.advance_timers(t);
        let c = e.control;
        let ci = c.index();
        let slot = e.device.slot();
        let pos = e.pos.unwrap_or([0.0; 2]);
        let before = self.slot_value[slot][ci];
        match c.kind() {
            ControlKind::Button => {
                let v = e.value.clamp(0.0, 1.0);
                self.slot_value[slot][ci] = v;
                let kind = e.device.kind().index();
                let (was, is) = (before >= PAD_ACTIVE, v >= PAD_ACTIVE);
                if is && !was {
                    self.held[kind] += 1;
                } else if was && !is {
                    self.held[kind] = self.held[kind].saturating_sub(1);
                }
                self.value[ci] = (0..DEVICE_SLOTS)
                    .map(|s| self.slot_value[s][ci])
                    .fold(0.0, f32::max);
            }
            ControlKind::Stick => {
                let p = [pos[0].clamp(-1.0, 1.0), pos[1].clamp(-1.0, 1.0)];
                self.slot_pos[slot][ci] = p;
                let best = (0..DEVICE_SLOTS)
                    .map(|s| self.slot_pos[s][ci])
                    .max_by(|a, b| len2(*a).total_cmp(&len2(*b)))
                    .unwrap_or([0.0; 2]);
                self.pos[ci] = best;
            }
            ControlKind::Delta => {
                self.pos[ci] = [self.pos[ci][0] + pos[0], self.pos[ci][1] + pos[1]];
                self.touch_delta(c);
            }
            ControlKind::Wheel => {
                self.value[ci] += e.value;
                self.touch_delta(c);
            }
            ControlKind::Position => {
                self.slot_pos[slot][ci] = pos;
                self.pos[ci] = pos;
            }
        }
        self.track_device(&e, before, t);
        for i in 0..self.by_control[ci].len() {
            let a = self.bindings[self.by_control[ci][i]].action;
            if self.actions[a].ty == ActionType::Bool {
                self.recompute_bool(a, t);
            }
        }
        self.clock = t;
    }

    fn touch_delta(&mut self, c: Control) {
        if !self.delta_flag[c.index()] {
            self.delta_flag[c.index()] = true;
            self.delta_touched.push(c);
        }
    }

    /// The last-used-device rule (spec 4.4).
    fn track_device(&mut self, e: &RawEvent, before: f32, t: f64) {
        let kind = e.device.kind();
        let pos = e.pos.unwrap_or([0.0; 2]);
        let meaningful = match (kind, e.control.kind()) {
            (DeviceKind::Touch, ControlKind::Button) => e.value > 0.0 && before <= 0.0,
            (DeviceKind::Touch, ControlKind::Stick) => len2(pos) > 0.0,
            (_, ControlKind::Button) => e.value >= PAD_ACTIVE && before < PAD_ACTIVE,
            (_, ControlKind::Stick) => len2(pos) > PAD_ACTIVE,
            (_, ControlKind::Wheel) => e.value != 0.0,
            (DeviceKind::KeyboardMouse, ControlKind::Delta | ControlKind::Position) => {
                let moved = if e.control.kind() == ControlKind::Delta {
                    len2(pos)
                } else {
                    let d = self
                        .last_pointer
                        .map_or(0.0, |p| len2([pos[0] - p[0], pos[1] - p[1]]));
                    self.last_pointer = Some(pos);
                    d
                };
                if t - self.mouse_since > MOUSE_MOVE_WINDOW {
                    self.mouse_since = t;
                    self.mouse_acc = 0.0;
                }
                self.mouse_acc += moved;
                self.mouse_acc >= MOUSE_MOVE_PIXELS
            }
            _ => false,
        };
        if !meaningful {
            return;
        }
        let current = self.last_device;
        if kind != current {
            let quiet = t - self.last_meaningful[current.index()] >= SWITCH_QUIET - EPS;
            let free = self.held[current.index()] == 0;
            let spaced = t - self.last_switch >= SWITCH_GAP - EPS;
            if !(quiet && free && spaced) {
                return;
            }
            self.last_device = kind;
            self.last_switch = t;
            self.device_changes += 1;
        }
        self.last_meaningful[kind.index()] = t;
    }

    /// One binding's value for its action's type.
    fn binding_value(&self, bi: usize) -> [f32; 3] {
        const ZERO: [f32; 3] = [0.0; 3];
        let b = &self.bindings[bi];
        let ty = self.actions[b.action].ty;
        let def = &b.def;
        for m in def.modifiers.iter().flatten() {
            if self.read(*m, b.rank) < PAD_ACTIVE {
                return ZERO;
            }
        }
        let mut v = [0.0f32; 3];
        let composite = def.composite.iter().any(Option::is_some);
        let mut is_stick = false;
        if composite {
            let k = |i: usize| def.composite[i].map_or(0.0, |c| self.read(c, b.rank));
            let (up, down, left, right, forward, backward) = (k(0), k(1), k(2), k(3), k(4), k(5));
            v = match ty {
                ActionType::Direction3D => [right - left, up - down, forward - backward],
                ActionType::Direction1D => [up - down, 0.0, 0.0],
                _ => [right - left, up - down, 0.0],
            };
            let len = len3(v);
            if len > 1.0 {
                v = [v[0] / len, v[1] / len, v[2] / len];
            }
        } else if let Some(c) = def.key {
            match c.kind() {
                ControlKind::Button | ControlKind::Wheel => v[0] = self.read(c, b.rank),
                kind => {
                    let p = self.read_pos(c, b.rank);
                    v = [p[0], p[1], 0.0];
                    is_stick = kind == ControlKind::Stick;
                }
            }
        } else {
            return ZERO;
        }
        if ty == ActionType::ViewportPosition {
            return v;
        }
        if ty == ActionType::Direction1D {
            v = [v[0], 0.0, 0.0];
        }
        // Dead zone: radial, rescaled so the output starts from 0.
        let dz = b.deadzone;
        if dz > 0.0 {
            let len = len3(v);
            if len < dz || !len.is_finite() {
                return ZERO;
            }
            let scaled = ((len - dz) / (1.0 - dz)).min(1.0) / len;
            v = [v[0] * scaled, v[1] * scaled, v[2] * scaled];
        }
        if def.response_curve != 1.0 {
            for x in &mut v {
                *x = x.signum() * x.abs().powf(def.response_curve);
            }
        }
        let axis_scale = match ty {
            ActionType::Direction2D => [def.vector2_scale[0], def.vector2_scale[1], 1.0],
            ActionType::Direction3D => def.vector3_scale,
            _ => [1.0; 3],
        };
        let dt = if def.scale_by_dt { self.dt } else { 1.0 };
        for i in 0..3 {
            v[i] *= def.scale * axis_scale[i] * dt;
        }
        if def.clamp_magnitude
            && (composite || is_stick)
            && matches!(ty, ActionType::Direction2D | ActionType::Direction3D)
        {
            let len = len3(v);
            if len > 1.0 {
                v = [v[0] / len, v[1] / len, v[2] / len];
            }
        }
        v
    }

    /// True when another binding of the same action, with modifiers that are down, reads one of
    /// this binding's controls: `Shift+W` beats a bare `W` (spec 3.4).
    fn shadowed(&self, bi: usize) -> bool {
        let b = &self.bindings[bi];
        if b.def.modifiers.iter().any(Option::is_some) {
            return false;
        }
        let range = self.actions[b.action].bindings.clone();
        range.filter(|&o| o != bi).any(|o| {
            let other = &self.bindings[o];
            other.def.modifiers.iter().any(Option::is_some)
                && other
                    .def
                    .modifiers
                    .iter()
                    .flatten()
                    .all(|&m| self.read(m, other.rank) >= PAD_ACTIVE)
                && other
                    .def
                    .key
                    .into_iter()
                    .chain(other.def.composite.iter().flatten().copied())
                    .any(|c| {
                        b.def
                            .key
                            .into_iter()
                            .chain(b.def.composite.iter().flatten().copied())
                            .any(|d| d == c)
                    })
        })
    }

    fn recompute_bool(&mut self, a: usize, t: f64) {
        if !self.actions[a].active {
            return;
        }
        let range = self.actions[a].bindings.clone();
        let mut down = false;
        for bi in range {
            let v = len3(if self.shadowed(bi) {
                [0.0; 3]
            } else {
                self.binding_value(bi)
            });
            let b = &mut self.bindings[bi];
            b.pressed = if b.pressed {
                v >= b.def.released_threshold
            } else {
                v >= b.def.pressed_threshold
            } && v > 0.0;
            down |= b.pressed;
        }
        if down != self.actions[a].down {
            if down {
                self.press(a, t);
            } else {
                self.release(a, t);
            }
        }
    }

    fn recompute_vector(&mut self, a: usize) {
        let range = self.actions[a].bindings.clone();
        let mut best = [0.0f32; 3];
        let mut best_len = 0.0f32;
        for bi in range {
            let v = if self.shadowed(bi) {
                [0.0; 3]
            } else {
                self.binding_value(bi)
            };
            let len = len3(v);
            if len > best_len {
                best_len = len;
                best = v;
            }
        }
        self.actions[a].value = best;
    }

    fn announce_vector(&mut self, a: usize, now: f64) {
        let act = &mut self.actions[a];
        if act.value == act.emitted {
            return;
        }
        act.emitted = act.value;
        let value = self.state(ActionId(a as u16));
        self.emit(a, ActionEventKind::StateChanged, value, now);
        self.emit(a, ActionEventKind::Triggered, value, now);
    }

    fn emit(&mut self, a: usize, kind: ActionEventKind, value: ActionValue, time: f64) {
        if self.events.len() == EVENT_CAPACITY {
            self.events.pop_front();
            self.dropped_events += 1;
        }
        self.events.push_back(ActionEvent {
            action: ActionId(a as u16),
            kind,
            value,
            time,
        });
    }

    fn press(&mut self, a: usize, t: f64) {
        let act = &mut self.actions[a];
        act.down = true;
        act.down_since = t;
        act.hold_fired = false;
        act.value[0] = 1.0;
        act.emitted[0] = 1.0;
        let trigger = act.trigger;
        if matches!(trigger, Trigger::Hold | Trigger::Repeat) {
            self.timers_active += 1;
            self.actions[a].next_repeat = t + self.actions[a].repeat_delay;
        }
        self.emit(a, ActionEventKind::Pressed, ActionValue::Bool(true), t);
        self.emit(a, ActionEventKind::StateChanged, ActionValue::Bool(true), t);
        if matches!(trigger, Trigger::Press | Trigger::Repeat) {
            self.emit(a, ActionEventKind::Triggered, ActionValue::Bool(true), t);
        }
    }

    fn release(&mut self, a: usize, t: f64) {
        let act = &mut self.actions[a];
        let held = t - act.down_since;
        let was_hold_or_repeat = matches!(act.trigger, Trigger::Hold | Trigger::Repeat);
        act.down = false;
        act.value[0] = 0.0;
        act.emitted[0] = 0.0;
        act.next_repeat = f64::INFINITY;
        let trigger = act.trigger;
        let is_tap = held <= act.tap + EPS;
        let mut fire = false;
        match trigger {
            Trigger::Release => fire = true,
            Trigger::Tap => fire = is_tap,
            Trigger::DoubleTap => {
                if is_tap {
                    if act.down_since - act.last_tap_release <= act.double_tap + EPS {
                        fire = true;
                        act.last_tap_release = f64::NEG_INFINITY;
                    } else {
                        act.last_tap_release = t;
                    }
                } else {
                    act.last_tap_release = f64::NEG_INFINITY;
                }
            }
            _ => {}
        }
        if was_hold_or_repeat {
            self.timers_active = self.timers_active.saturating_sub(1);
        }
        self.emit(a, ActionEventKind::Released, ActionValue::Bool(false), t);
        self.emit(
            a,
            ActionEventKind::StateChanged,
            ActionValue::Bool(false),
            t,
        );
        if fire {
            self.emit(a, ActionEventKind::Triggered, ActionValue::Bool(true), t);
        }
    }

    /// Fires the triggers that wait for time: Hold, and Repeat while down.
    fn advance_timers(&mut self, t: f64) {
        if self.timers_active == 0 {
            return;
        }
        for i in 0..self.timed.len() {
            let a = self.timed[i];
            if !self.actions[a].down {
                continue;
            }
            match self.actions[a].trigger {
                Trigger::Hold => {
                    let at = self.actions[a].down_since + self.actions[a].hold;
                    if !self.actions[a].hold_fired && t + EPS >= at {
                        self.actions[a].hold_fired = true;
                        self.emit(a, ActionEventKind::Triggered, ActionValue::Bool(true), at);
                    }
                }
                Trigger::Repeat => {
                    let mut fired = 0;
                    while t + EPS >= self.actions[a].next_repeat && fired < MAX_REPEATS_PER_CALL {
                        let at = self.actions[a].next_repeat;
                        self.actions[a].next_repeat += self.actions[a].repeat_rate;
                        fired += 1;
                        self.emit(a, ActionEventKind::Triggered, ActionValue::Bool(true), at);
                    }
                    if fired == MAX_REPEATS_PER_CALL {
                        // A long pause: do not catch up, carry on from now.
                        self.actions[a].next_repeat = t + self.actions[a].repeat_rate;
                    }
                }
                _ => {}
            }
        }
    }
}
