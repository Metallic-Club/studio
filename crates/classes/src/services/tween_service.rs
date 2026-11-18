//! `TweenService` (#810): properties moved smoothly over time, as Roblox's.
//!
//! `TweenService:Create(instance, tweenInfo, goals)` makes a [`Tween`](crate::services::tween) for the instance;
//! `Play` starts it. What a tween needs between frames (its goals, the values it started from, how far it is) is
//! the game's [`Tweens`], kept in the data model's state; the engine moves every playing tween once a frame with
//! [`step`]. Numbers, `Vector2`, `Vector3`, `CFrame`, `Color3` and booleans can be tweened; a boolean takes its goal
//! when the tween reaches it.
//!
//! As Roblox's: a tween starts from the values the properties have when it is played; `DelayTime` is waited before
//! each play; `RepeatCount` plays it again (for ever when negative), from the start values, or back to them when it
//! `Reverses`; a tween played on a property another tween is playing cancels that one. `Completed` fires with
//! `Enum.PlaybackState.Completed` at the end, or `Cancelled` when cancelled.
//!
//! Differences: a tween's `TweenInfo` cannot be read back from it, and a finished tween stays in the game until it
//! is destroyed (`Destroy`), as nothing tells the engine that no script holds it any more.

use std::f64::consts::PI;

use metallic_core::{
    ClassDesc, DataModel, EnumDesc, EnumItem, Id, MethodCall, MethodDesc, Param, PropId, PropType, ScriptError,
    ScriptValue, Ty, TweenInfo, Value,
};

/// Smooth property changes.
pub static CLASS: ClassDesc = ClassDesc::new("TweenService", Some("Instance"))
    .describe("Makes tweens: properties of an instance moved smoothly to goals over time, along an easing curve.")
    .service(12)
    .hidden_in_explorer()
    .methods(METHODS);

/// `Enum.EasingStyle`, `Enum.EasingDirection` and `Enum.PlaybackState`, in Roblox's order (and so its numbers).
pub static ENUMS: &[EnumDesc] = &[
    EnumDesc::new(
        "EasingStyle",
        &[
            "Linear",
            "Sine",
            "Back",
            "Quad",
            "Quart",
            "Quint",
            "Bounce",
            "Elastic",
            "Exponential",
            "Circular",
            "Cubic",
        ],
    )
    .describe("The curve a tween follows from its start to its goal."),
    EnumDesc::new("EasingDirection", &["In", "Out", "InOut"])
        .describe("Which end of a tween's curve is eased: the start (In), the end (Out) or both (InOut)."),
    EnumDesc::new(
        "PlaybackState",
        &["Begin", "Delayed", "Playing", "Paused", "Completed", "Cancelled"],
    )
    .describe("Where a tween is: not yet played, waiting its delay, playing, paused, finished or cancelled."),
];

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("Create", create)
        .params(&[
            Param::new("instance", Ty::Class("Instance")),
            Param::new("tweenInfo", Ty::Data("TweenInfo")),
            Param::new("propertyTable", Ty::Map(&Ty::String, &Ty::Any)),
        ])
        .returns(Ty::Class("Tween"))
        .describe(
            "Makes a tween that moves the instance's properties to the values in propertyTable (property names to \
             goals), as tweenInfo says. Call Play on it to start it.",
        ),
    MethodDesc::new("GetValue", get_value)
        .params(&[
            Param::new("alpha", Ty::Number),
            Param::new("easingStyle", Ty::Enum("EasingStyle")),
            Param::new("easingDirection", Ty::Enum("EasingDirection")),
        ])
        .returns(Ty::Number)
        .describe("Returns where an eased tween is, from 0 to 1 (Back and Elastic go past), when it is alpha of the way."),
];

/// A curve, `Enum.EasingStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// `Linear`.
    Linear,
    /// `Sine`.
    Sine,
    /// `Back`.
    Back,
    /// `Quad`.
    Quad,
    /// `Quart`.
    Quart,
    /// `Quint`.
    Quint,
    /// `Bounce`.
    Bounce,
    /// `Elastic`.
    Elastic,
    /// `Exponential`.
    Exponential,
    /// `Circular`.
    Circular,
    /// `Cubic`.
    Cubic,
}

impl Style {
    /// The style of an `Enum.EasingStyle` item.
    pub fn of(item: &EnumItem) -> Option<Self> {
        if item.enum_type() != "EasingStyle" {
            return None;
        }
        Some(match item.name() {
            "Linear" => Self::Linear,
            "Sine" => Self::Sine,
            "Back" => Self::Back,
            "Quad" => Self::Quad,
            "Quart" => Self::Quart,
            "Quint" => Self::Quint,
            "Bounce" => Self::Bounce,
            "Elastic" => Self::Elastic,
            "Exponential" => Self::Exponential,
            "Circular" => Self::Circular,
            "Cubic" => Self::Cubic,
            _ => return None,
        })
    }

    /// The eased-in curve at `t` (0 to 1).
    fn ease_in(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::Sine => 1.0 - (t * PI / 2.0).cos(),
            Self::Quad => t * t,
            Self::Cubic => t * t * t,
            Self::Quart => t * t * t * t,
            Self::Quint => t * t * t * t * t,
            Self::Exponential => {
                if t <= 0.0 {
                    0.0
                } else {
                    2f64.powf(10.0 * t - 10.0)
                }
            }
            Self::Circular => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            Self::Back => {
                const S: f64 = 1.70158;
                t * t * ((S + 1.0) * t - S)
            }
            Self::Elastic => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    -(2f64.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * (2.0 * PI / 3.0)).sin()
                }
            }
            Self::Bounce => 1.0 - bounce_out(1.0 - t),
        }
    }
}

/// The bounce curve, eased out.
fn bounce_out(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

/// Which end is eased, `Enum.EasingDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// `In`: the start.
    In,
    /// `Out`: the end.
    Out,
    /// `InOut`: both.
    InOut,
}

impl Direction {
    /// The direction of an `Enum.EasingDirection` item.
    pub fn of(item: &EnumItem) -> Option<Self> {
        if item.enum_type() != "EasingDirection" {
            return None;
        }
        Some(match item.name() {
            "In" => Self::In,
            "Out" => Self::Out,
            "InOut" => Self::InOut,
            _ => return None,
        })
    }
}

/// Where a tween eased by `style` and `direction` is when it is `alpha` (0 to 1, clamped) of the way.
pub fn ease(style: Style, direction: Direction, alpha: f64) -> f64 {
    let t = alpha.clamp(0.0, 1.0);
    match direction {
        Direction::In => style.ease_in(t),
        Direction::Out => 1.0 - style.ease_in(1.0 - t),
        Direction::InOut => {
            if t < 0.5 {
                style.ease_in(2.0 * t) / 2.0
            } else {
                1.0 - style.ease_in(2.0 - 2.0 * t) / 2.0
            }
        }
    }
}

fn get_value(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let alpha = call.args.number(0)?;
    let style = Style::of(&call.args.enum_item(1)?)
        .ok_or_else(|| ScriptError::bad_argument(2, "Enum.EasingStyle", "EnumItem"))?;
    let direction = Direction::of(&call.args.enum_item(2)?)
        .ok_or_else(|| ScriptError::bad_argument(3, "Enum.EasingDirection", "EnumItem"))?;
    Ok(ScriptValue::Number(ease(style, direction, alpha)))
}

/// Where a tween is, `Enum.PlaybackState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Not played yet.
    Begin,
    /// Waiting its delay.
    Delayed,
    /// Moving.
    Playing,
    /// Paused by `Pause`.
    Paused,
    /// At its end.
    Completed,
    /// Stopped by `Cancel`, or by another tween of the same property.
    Cancelled,
}

impl State {
    /// Its `Enum.PlaybackState` item.
    pub fn item(self) -> EnumItem {
        let name = match self {
            Self::Begin => "Begin",
            Self::Delayed => "Delayed",
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
        };
        EnumItem::new("PlaybackState", name)
    }

    /// Whether it is moving or waiting to.
    fn active(self) -> bool {
        matches!(self, Self::Delayed | Self::Playing)
    }
}

/// One tween's playback.
#[derive(Clone, Debug)]
pub struct TweenRecord {
    /// The instance it moves.
    pub instance: Id,
    time: f64,
    style: Style,
    direction: Direction,
    repeat_count: i32,
    reverses: bool,
    delay_time: f64,
    /// Each property and its goal.
    goals: Vec<(PropId, Value)>,
    /// Each property's value when it was played, in the order of `goals`.
    starts: Vec<Value>,
    /// Seconds since it was played, pauses left out.
    elapsed: f64,
    /// Where it is.
    pub state: State,
}

impl TweenRecord {
    /// Seconds one play takes: the delay, there, and back when it reverses.
    fn cycle(&self) -> f64 {
        self.delay_time + self.time * if self.reverses { 2.0 } else { 1.0 }
    }

    /// Where the properties are at `elapsed`: `None` while waiting a delay (they stay), else how far along the
    /// curve (eased) and along the time (not eased, for booleans), and whether this is the end.
    fn at(&self, elapsed: f64) -> (Option<(f64, f64)>, bool) {
        let cycle = self.cycle();
        let plays = if self.repeat_count < 0 {
            f64::INFINITY
        } else {
            f64::from(self.repeat_count) + 1.0
        };
        if cycle <= 0.0 || elapsed >= cycle * plays {
            let end = if self.reverses { 0.0 } else { 1.0 };
            return (Some((end, end)), true);
        }
        let within = elapsed - (elapsed / cycle).floor() * cycle;
        if within < self.delay_time {
            return (None, false);
        }
        let t = within - self.delay_time;
        let linear = if self.time <= 0.0 {
            1.0
        } else if t < self.time {
            t / self.time
        } else {
            1.0 - (t - self.time) / self.time
        };
        (Some((ease(self.style, self.direction, linear), linear)), false)
    }
}

/// The game's tweens, in the order they were made: what [`step`] moves and the tween methods change.
#[derive(Debug, Default)]
pub struct Tweens {
    /// Each tween and its playback.
    pub tweens: Vec<(Id, TweenRecord)>,
}

impl Tweens {
    /// The playback of `tween`.
    pub fn get_mut(&mut self, tween: Id) -> Option<&mut TweenRecord> {
        self.tweens.iter_mut().find(|(id, _)| *id == tween).map(|(_, record)| record)
    }
}

/// Whether a property of type `ty` can be tweened.
fn tweenable(ty: &PropType) -> bool {
    matches!(
        ty,
        PropType::Number
            | PropType::Int
            | PropType::Vector2
            | PropType::Vector3
            | PropType::CFrame
            | PropType::Color3
            | PropType::Bool
    )
}

fn create(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let instance = call.args.instance(0)?;
    let info: TweenInfo = match call.args.get(1) {
        Some(ScriptValue::TweenInfo(info)) => (**info).clone(),
        other => {
            return Err(ScriptError::bad_argument(
                2,
                "TweenInfo",
                other.map_or("no value", ScriptValue::type_name),
            ));
        }
    };
    let goals = match call.args.get(2) {
        Some(ScriptValue::Map(goals)) => goals.clone(),
        Some(ScriptValue::List(list)) if list.is_empty() => Vec::new(),
        other => {
            return Err(ScriptError::bad_argument(
                3,
                "a table of property names to goals",
                other.map_or("no value", ScriptValue::type_name),
            ));
        }
    };
    let dm = &mut *call.dm;
    let class = dm.class_of(instance)?;
    let mut converted = Vec::with_capacity(goals.len());
    for (name, goal) in &goals {
        let prop = dm.prop_id(instance, name)?;
        let ty = dm
            .registry()
            .prop(class, prop)
            .map(|resolved| resolved.meta.ty)
            .ok_or_else(|| ScriptError::runtime(format!("{name} is not a property")))?;
        if !tweenable(&ty) {
            return Err(ScriptError::runtime(format!(
                "TweenService:Create property named '{name}' cannot be tweened due to type mismatch (property is a \
                 '{ty}', but given type is '{}')",
                goal.type_name()
            )));
        }
        let value = goal.to_value(&ty).map_err(|_| {
            ScriptError::runtime(format!(
                "TweenService:Create property named '{name}' cannot be tweened due to type mismatch (property is a \
                 '{ty}', but given type is '{}')",
                goal.type_name()
            ))
        })?;
        converted.push((prop, value));
    }
    let style = Style::of(&info.easing_style).unwrap_or(Style::Quad);
    let direction = Direction::of(&info.easing_direction).unwrap_or(Direction::Out);
    let tween_class = dm
        .registry()
        .class_id(super::tween::CLASS.name)
        .ok_or_else(|| ScriptError::runtime("there is no Tween class"))?;
    let tween = dm.create_internal(tween_class)?;
    dm.set_by_name(tween, "Instance", Value::Ref(Some(instance)))?;
    let mut tweens = dm.take_state::<Tweens>().unwrap_or_default();
    // Tweens destroyed since are forgotten here, so the list only holds what scripts can still reach.
    tweens.tweens.retain(|(id, _)| dm.contains(*id));
    tweens.tweens.push((
        tween,
        TweenRecord {
            instance,
            time: info.time.max(0.0),
            style,
            direction,
            repeat_count: info.repeat_count,
            reverses: info.reverses,
            delay_time: info.delay_time.max(0.0),
            goals: converted,
            starts: Vec::new(),
            elapsed: 0.0,
            state: State::Begin,
        },
    ));
    dm.put_state(tweens);
    Ok(ScriptValue::Instance(tween))
}

/// `Play`: from the start (taking the properties' values now), or on from a pause.
pub(crate) fn play(dm: &mut DataModel, tween: Id) -> Result<(), ScriptError> {
    let mut tweens = dm.take_state::<Tweens>().unwrap_or_default();
    let mut cancelled = Vec::new();
    if let Some(index) = tweens.tweens.iter().position(|(id, _)| *id == tween) {
        let (instance, props, state) = {
            let record = &tweens.tweens[index].1;
            let props: Vec<PropId> = record.goals.iter().map(|(p, _)| *p).collect();
            (record.instance, props, record.state)
        };
        match state {
            State::Playing | State::Delayed => {}
            State::Paused => {
                let record = &mut tweens.tweens[index].1;
                record.state = State::Playing;
            }
            State::Begin | State::Completed | State::Cancelled => {
                // Another tween of the same property gives way.
                for (other, record) in &mut tweens.tweens {
                    if *other != tween
                        && record.instance == instance
                        && record.state.active()
                        && record.goals.iter().any(|(p, _)| props.contains(p))
                    {
                        record.state = State::Cancelled;
                        cancelled.push(*other);
                    }
                }
                let starts: Vec<Value> = props
                    .iter()
                    .map(|&prop| dm.get(instance, prop))
                    .collect::<Result<_, _>>()?;
                let record = &mut tweens.tweens[index].1;
                record.starts = starts;
                record.elapsed = 0.0;
                record.state = if record.delay_time > 0.0 {
                    State::Delayed
                } else {
                    State::Playing
                };
            }
        }
    }
    let states: Vec<(Id, State)> = tweens.tweens.iter().map(|(id, r)| (*id, r.state)).collect();
    dm.put_state(tweens);
    for (id, state) in states.into_iter().filter(|(id, _)| *id == tween || cancelled.contains(id)) {
        write_state(dm, id, state);
    }
    for other in cancelled {
        completed(dm, other, State::Cancelled);
    }
    Ok(())
}

/// `Pause`: stops it where it is, until it is played again.
pub(crate) fn pause(dm: &mut DataModel, tween: Id) {
    let mut tweens = dm.take_state::<Tweens>().unwrap_or_default();
    let paused = tweens.get_mut(tween).is_some_and(|record| {
        let active = record.state.active();
        if active {
            record.state = State::Paused;
        }
        active
    });
    dm.put_state(tweens);
    if paused {
        write_state(dm, tween, State::Paused);
    }
}

/// `Cancel`: stops it where it is; playing it again starts it over.
pub(crate) fn cancel(dm: &mut DataModel, tween: Id) {
    let mut tweens = dm.take_state::<Tweens>().unwrap_or_default();
    let stopped = tweens.get_mut(tween).is_some_and(|record| {
        let running = record.state.active() || record.state == State::Paused;
        if running {
            record.state = State::Cancelled;
            record.elapsed = 0.0;
        }
        running
    });
    dm.put_state(tweens);
    if stopped {
        write_state(dm, tween, State::Cancelled);
        completed(dm, tween, State::Cancelled);
    }
}

/// Shows `state` in the tween's `PlaybackState`.
fn write_state(dm: &mut DataModel, tween: Id, state: State) {
    // Only a destroyed tween fails, and nothing reads it then.
    let _ = dm.set_by_name(tween, "PlaybackState", Value::Enum(state.item()));
}

/// Fires the tween's `Completed` with `state`.
fn completed(dm: &mut DataModel, tween: Id, state: State) {
    if let Ok(signal) = dm.signal_id(tween, "Completed") {
        let _ = dm.fire(tween, signal, vec![ScriptValue::Enum(state.item())]);
    }
}

/// `start` moved toward `goal` by `alpha` (eased) or, for a boolean, `linear` (it switches at the goal).
fn between(start: &Value, goal: &Value, alpha: f64, linear: f64) -> Value {
    let a = alpha as f32;
    match (start, goal) {
        (Value::Number(s), Value::Number(g)) => Value::Number(s + (g - s) * alpha),
        (Value::Int(s), Value::Int(g)) => Value::Int((*s as f64 + (*g as f64 - *s as f64) * alpha).round() as i64),
        (Value::Vector2(s), Value::Vector2(g)) => Value::Vector2(*s + (*g - *s) * a),
        (Value::Vector3(s), Value::Vector3(g)) => Value::Vector3(*s + (*g - *s) * a),
        (Value::CFrame(s), Value::CFrame(g)) => Value::CFrame(s.lerp(*g, a)),
        (Value::Color3(s), Value::Color3(g)) => Value::Color3(s.lerp(*g, a)),
        (Value::Bool(_), Value::Bool(g)) if linear >= 1.0 => Value::Bool(*g),
        _ => start.clone(),
    }
}

/// Moves every playing tween on by `dt` seconds, and fires `Completed` for those that reached their end. The engine
/// calls it once a frame.
pub fn step(dm: &mut DataModel, dt: f64) {
    // Most frames of most games: no tween is playing, and nothing is taken out or put back.
    if !dm
        .state::<Tweens>()
        .is_some_and(|tweens| tweens.tweens.iter().any(|(_, record)| record.state.active()))
    {
        return;
    }
    let Some(mut tweens) = dm.take_state::<Tweens>() else {
        return;
    };
    let mut writes: Vec<(Id, PropId, Value)> = Vec::new();
    let mut changed: Vec<(Id, State)> = Vec::new();
    let mut finished = Vec::new();
    tweens.tweens.retain(|(id, _)| dm.contains(*id));
    for (tween, record) in &mut tweens.tweens {
        if !record.state.active() {
            continue;
        }
        if !dm.contains(record.instance) {
            record.state = State::Cancelled;
            changed.push((*tween, State::Cancelled));
            finished.push((*tween, State::Cancelled));
            continue;
        }
        record.elapsed += dt.max(0.0);
        let (at, done) = record.at(record.elapsed);
        let state = match (at, done) {
            (_, true) => State::Completed,
            (None, false) => State::Delayed,
            (Some(_), false) => State::Playing,
        };
        if let Some((alpha, linear)) = at {
            for ((prop, goal), start) in record.goals.iter().zip(&record.starts) {
                writes.push((record.instance, *prop, between(start, goal, alpha, linear)));
            }
        }
        if state != record.state {
            record.state = state;
            changed.push((*tween, state));
        }
        if done {
            finished.push((*tween, State::Completed));
        }
    }
    dm.put_state(tweens);
    for (instance, prop, value) in writes {
        // A property a script made unwritable meanwhile (a destroyed instance) is left.
        let _ = dm.set(instance, prop, value);
    }
    for (tween, state) in changed {
        write_state(dm, tween, state);
    }
    for (tween, state) in finished {
        completed(dm, tween, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curves_start_at_0_and_end_at_1() {
        let styles = [
            Style::Linear,
            Style::Sine,
            Style::Back,
            Style::Quad,
            Style::Quart,
            Style::Quint,
            Style::Bounce,
            Style::Elastic,
            Style::Exponential,
            Style::Circular,
            Style::Cubic,
        ];
        for style in styles {
            for direction in [Direction::In, Direction::Out, Direction::InOut] {
                assert!(ease(style, direction, 0.0).abs() < 1e-3, "{style:?} {direction:?}");
                assert!((ease(style, direction, 1.0) - 1.0).abs() < 1e-3, "{style:?} {direction:?}");
            }
        }
        // Known points.
        assert!((ease(Style::Quad, Direction::In, 0.5) - 0.25).abs() < 1e-9);
        assert!((ease(Style::Quad, Direction::Out, 0.5) - 0.75).abs() < 1e-9);
        assert!((ease(Style::Cubic, Direction::InOut, 0.25) - 0.0625).abs() < 1e-9);
        assert!((ease(Style::Linear, Direction::Out, 0.3) - 0.3).abs() < 1e-9);
        assert!((ease(Style::Sine, Direction::In, 0.5) - (1.0 - (PI / 4.0).cos())).abs() < 1e-9);
        // Back dips below the start; Elastic overshoots the goal.
        assert!(ease(Style::Back, Direction::In, 0.2) < 0.0);
        assert!((0..100).any(|i| ease(Style::Elastic, Direction::Out, f64::from(i) / 100.0) > 1.0));
        assert!((ease(Style::Bounce, Direction::Out, 1.0 / 2.75) - 1.0).abs() < 1e-9);
    }

    fn record(time: f64, repeat_count: i32, reverses: bool, delay_time: f64) -> TweenRecord {
        TweenRecord {
            instance: Id::new(1, 0),
            time,
            style: Style::Linear,
            direction: Direction::In,
            repeat_count,
            reverses,
            delay_time,
            goals: Vec::new(),
            starts: Vec::new(),
            elapsed: 0.0,
            state: State::Playing,
        }
    }

    #[test]
    fn delays_repeats_and_reverses_follow_the_clock() {
        let once = record(2.0, 0, false, 0.0);
        assert_eq!(once.at(1.0), (Some((0.5, 0.5)), false));
        assert_eq!(once.at(2.0), (Some((1.0, 1.0)), true));

        let delayed = record(1.0, 0, false, 0.5);
        assert_eq!(delayed.at(0.25), (None, false));
        assert_eq!(delayed.at(1.0), (Some((0.5, 0.5)), false));

        // There and back, twice.
        let back = record(1.0, 1, true, 0.0);
        assert_eq!(back.at(0.5), (Some((0.5, 0.5)), false));
        assert_eq!(back.at(1.5), (Some((0.5, 0.5)), false));
        assert_eq!(back.at(2.25), (Some((0.25, 0.25)), false));
        assert_eq!(back.at(4.0), (Some((0.0, 0.0)), true));

        // For ever.
        let forever = record(1.0, -1, false, 0.0);
        assert!(!forever.at(1000.5).1);

        // No time: straight to the goal.
        assert_eq!(record(0.0, 0, false, 0.0).at(0.0), (Some((1.0, 1.0)), true));
    }

    #[test]
    fn values_move_between_start_and_goal() {
        assert_eq!(
            between(&Value::Number(0.0), &Value::Number(10.0), 0.25, 0.25),
            Value::Number(2.5)
        );
        assert_eq!(between(&Value::Bool(false), &Value::Bool(true), 0.9, 0.9), Value::Bool(false));
        assert_eq!(between(&Value::Bool(false), &Value::Bool(true), 1.0, 1.0), Value::Bool(true));
    }
}
