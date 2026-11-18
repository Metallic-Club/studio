//! `Tween` (#810): one tween `TweenService:Create` made, as Roblox's: `Play`, `Pause`, `Cancel`, its
//! `PlaybackState` and `Completed`. Its playback lives in the game's `Tweens` (see `tween_service`).

use metallic_core::{
    ClassDesc, MethodCall, MethodDesc, Param, PropFlags, PropMeta, ScriptError, ScriptValue, SignalDesc, Ty,
};

use super::tween_service;

/// Set only by the engine, never saved: a tween lives for one session.
const FIELD: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

/// One tween.
pub static CLASS: ClassDesc = ClassDesc::new("Tween", Some("Instance"))
    .describe("Moves an instance's properties to goals over time. Made by TweenService:Create; started with Play.")
    .props(PROPS)
    .methods(METHODS)
    .signals(&[SignalDesc::new("Completed")
        .params(&[Param::new("playbackState", Ty::Enum("PlaybackState"))])
        .describe("Fires when the tween reaches its end (Completed) or is cancelled (Cancelled).")]);

static PROPS: &[PropMeta] = &[
    PropMeta::reference("Instance", "Instance")
        .flags(FIELD)
        .group("Tween")
        .order(0)
        .desc("The instance whose properties the tween moves."),
    PropMeta::enumeration("PlaybackState", "PlaybackState", "Begin")
        .flags(FIELD)
        .group("Tween")
        .order(1)
        .desc("Where the tween is: Begin, Delayed, Playing, Paused, Completed or Cancelled."),
];

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("Play", play)
        .params(&[])
        .returns_nothing()
        .describe(
            "Starts the tween from the properties' values now, or goes on from where Pause stopped it. A playing \
             tween of the same properties is cancelled.",
        ),
    MethodDesc::new("Pause", pause)
        .params(&[])
        .returns_nothing()
        .describe("Stops the tween where it is; Play goes on from there."),
    MethodDesc::new("Cancel", cancel)
        .params(&[])
        .returns_nothing()
        .describe("Stops the tween where it is and fires Completed with Cancelled; Play starts it over."),
];

fn play(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    tween_service::play(call.dm, call.this)?;
    Ok(ScriptValue::Nil)
}

fn pause(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    tween_service::pause(call.dm, call.this);
    Ok(ScriptValue::Nil)
}

fn cancel(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    tween_service::cancel(call.dm, call.this);
    Ok(ScriptValue::Nil)
}
