//! `Sound` (#811): a sound file played by the engine, as Roblox's.
//!
//! A sound plays while `Playing` is true and it is in the game (anywhere under it), or after
//! `SoundService:PlayLocalSound` wherever it is. The methods only change properties and fire the events of their own
//! calls (`Played`, `Stopped`, `Paused`, `Resumed`); the engine (`metallic_engine`'s sounds) plays what the
//! properties say once a frame, and writes back what only it knows: `TimePosition` as it plays, `TimeLength` and
//! `IsLoaded` once the file is decoded, `IsPlaying`, and the `Loaded`, `DidLoop` and `Ended` events.
//!
//! As Roblox's: `Play` starts from `TimePosition` (from the start again when it is playing already), `Stop` goes back
//! to the start, `Pause` keeps the place and `Resume` goes on from it; at the end `Playing` turns false and
//! `TimePosition` goes back to 0. Sounds are not positional yet: one in a part sounds the same wherever the camera
//! is.

use metallic_core::{
    ClassBehavior, ClassDesc, DataModel, Id, MethodCall, MethodDesc, Param, PropFlags, PropMeta, ScriptError,
    ScriptValue, SignalDesc, Ty, Value, Widget,
};

/// The extensions of the sound files `SoundId` takes (`metallic_project`'s sound decoding reads them).
pub const SOUND_EXTENSIONS: &[&str] = &["wav", "ogg", "mp3", "flac"];

/// The group its properties are listed under.
const GROUP: &str = "Sound";

/// Written by the engine as the sound plays, never saved.
const STATE: PropFlags = PropFlags::READ_ONLY
    .union(PropFlags::NO_SAVE)
    .union(PropFlags::RUNTIME);

/// The name of the hidden count of `Play` calls, which tells the engine to start over.
pub const PLAY_COUNT: &str = "PlayCount";

/// The name of the hidden flag `SoundService:PlayLocalSound` sets: it plays wherever it is.
pub const PLAY_LOCAL: &str = "PlayLocal";

/// A sound file played by the engine.
pub static CLASS: ClassDesc = ClassDesc::new("Sound", Some("Instance"))
    .describe(
        "Plays a sound file: set SoundId and call Play, or set Playing. Volume, PlaybackSpeed and Looped change how \
         it plays.",
    )
    .category("Sounds")
    .creatable()
    .props(PROPS)
    .methods(METHODS)
    .signals(SIGNALS)
    .behavior(&Registers);

/// A sound's id in the events that pass it.
const SOUND_ID: Param = Param::new("soundId", Ty::String);

static PROPS: &[PropMeta] = &[
    PropMeta::string("SoundId", "")
        .widget(Widget::AssetId {
            extensions: SOUND_EXTENSIONS,
        })
        .group(GROUP)
        .order(0)
        .desc("The id of the project's sound asset (WAV, Ogg Vorbis, FLAC or MP3) it plays."),
    PropMeta::number("Volume", 0.5)
        .range(0.0, 10.0)
        .group(GROUP)
        .order(1)
        .desc("How loud it plays: 0 is silent, 1 as recorded; up to 10."),
    PropMeta::number("PlaybackSpeed", 1.0)
        .range(0.0, 20.0)
        .group(GROUP)
        .order(2)
        .desc("How fast it plays: 2 is twice as fast and an octave higher, 0.5 half as fast and an octave lower."),
    PropMeta::boolean("Looped", false)
        .group(GROUP)
        .order(3)
        .desc("Whether it starts over at the end instead of stopping."),
    PropMeta::boolean("Playing", false)
        .group(GROUP)
        .order(4)
        .desc("Whether it is playing. Setting it plays or pauses it; true when the game starts plays it then."),
    PropMeta::number("TimePosition", 0.0)
        .flags(PropFlags::NO_SAVE.union(PropFlags::RUNTIME))
        .group(GROUP)
        .order(5)
        .desc("Where it is, in seconds from the start. Setting it moves there."),
    PropMeta::number("TimeLength", 0.0)
        .flags(STATE)
        .group(GROUP)
        .order(6)
        .desc("How long the sound file plays, in seconds; 0 until it is loaded."),
    PropMeta::boolean("IsPlaying", false)
        .flags(STATE)
        .group(GROUP)
        .order(7)
        .desc("Whether it is being heard now."),
    PropMeta::boolean("IsPaused", false)
        .flags(STATE)
        .group(GROUP)
        .order(8)
        .desc("Whether it was paused partway, by Pause or by setting Playing to false."),
    PropMeta::boolean("IsLoaded", false)
        .flags(STATE)
        .group(GROUP)
        .order(9)
        .desc("Whether its sound file was loaded, so it can play at once."),
    PropMeta::int(PLAY_COUNT, 0)
        .flags(STATE.union(PropFlags::HIDDEN))
        .group(GROUP)
        .order(10)
        .desc("Engine bookkeeping: how many times Play was called, so a playing sound starts over."),
    PropMeta::boolean(PLAY_LOCAL, false)
        .flags(STATE.union(PropFlags::HIDDEN))
        .group(GROUP)
        .order(11)
        .desc("Engine bookkeeping: SoundService:PlayLocalSound played it, so it plays wherever it is."),
];

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("Play", play)
        .params(&[])
        .returns_nothing()
        .describe("Plays the sound from TimePosition, or from the start again when it is playing already."),
    MethodDesc::new("Stop", stop)
        .params(&[])
        .returns_nothing()
        .describe("Stops the sound and goes back to the start."),
    MethodDesc::new("Pause", pause)
        .params(&[])
        .returns_nothing()
        .describe("Stops the sound where it is; Resume or Play goes on from there."),
    MethodDesc::new("Resume", resume)
        .params(&[])
        .returns_nothing()
        .describe("Goes on playing from where Pause stopped it."),
];

static SIGNALS: &[SignalDesc] = &[
    SignalDesc::new("Played")
        .params(&[SOUND_ID])
        .describe("Fires when Play is called."),
    SignalDesc::new("Stopped")
        .params(&[SOUND_ID])
        .describe("Fires when Stop is called."),
    SignalDesc::new("Paused")
        .params(&[SOUND_ID])
        .describe("Fires when Pause is called."),
    SignalDesc::new("Resumed")
        .params(&[SOUND_ID])
        .describe("Fires when Resume is called."),
    SignalDesc::new("Ended")
        .params(&[SOUND_ID])
        .describe("Fires when the sound plays to its end (never while Looped)."),
    SignalDesc::new("Loaded")
        .params(&[SOUND_ID])
        .describe("Fires when its sound file was loaded."),
    SignalDesc::new("DidLoop")
        .params(&[SOUND_ID, Param::new("numOfTimesLooped", Ty::Number)])
        .describe("Fires each time a Looped sound starts over, with how many times it has."),
];

/// Every sound made, for the engine to look at each frame without walking the game: a list kept by the class's
/// `on_created` (in the data model's state); destroyed ones are dropped as the engine meets them.
#[derive(Debug, Default)]
pub struct SoundList {
    /// The sounds, in the order they were made.
    pub sounds: Vec<Id>,
}

/// Adds every new sound to the [`SoundList`].
struct Registers;

impl ClassBehavior for Registers {
    fn on_created(&self, dm: &mut DataModel, id: Id) {
        let mut list = dm.take_state::<SoundList>().unwrap_or_default();
        list.sounds.push(id);
        dm.put_state(list);
    }
}

/// Fires the sound's `signal` with its `SoundId`.
pub fn fire(dm: &mut DataModel, sound: Id, signal: &str) {
    let id = match dm.get_by_name(sound, "SoundId") {
        Ok(Value::String(id)) => id,
        _ => return,
    };
    if let Ok(signal) = dm.signal_id(sound, signal) {
        let _ = dm.fire(sound, signal, vec![ScriptValue::String(id)]);
    }
}

/// `Play`, for this class and `SoundService:PlayLocalSound`.
pub(crate) fn start(dm: &mut DataModel, sound: Id) -> Result<(), ScriptError> {
    if dm.get_by_name(sound, "Playing")? == Value::Bool(true) {
        dm.set_by_name(sound, "TimePosition", Value::Number(0.0))?;
    }
    let count = match dm.get_by_name(sound, PLAY_COUNT)? {
        Value::Int(count) => count,
        _ => 0,
    };
    dm.set_by_name(sound, PLAY_COUNT, Value::Int(count.wrapping_add(1)))?;
    dm.set_by_name(sound, "Playing", Value::Bool(true))?;
    dm.set_by_name(sound, "IsPaused", Value::Bool(false))?;
    fire(dm, sound, "Played");
    Ok(())
}

fn play(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    start(call.dm, call.this)?;
    Ok(ScriptValue::Nil)
}

fn stop(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let dm = &mut *call.dm;
    dm.set_by_name(call.this, "Playing", Value::Bool(false))?;
    dm.set_by_name(call.this, "TimePosition", Value::Number(0.0))?;
    dm.set_by_name(call.this, "IsPaused", Value::Bool(false))?;
    fire(dm, call.this, "Stopped");
    Ok(ScriptValue::Nil)
}

fn pause(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let dm = &mut *call.dm;
    dm.set_by_name(call.this, "Playing", Value::Bool(false))?;
    dm.set_by_name(call.this, "IsPaused", Value::Bool(true))?;
    fire(dm, call.this, "Paused");
    Ok(ScriptValue::Nil)
}

fn resume(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let dm = &mut *call.dm;
    dm.set_by_name(call.this, "Playing", Value::Bool(true))?;
    dm.set_by_name(call.this, "IsPaused", Value::Bool(false))?;
    fire(dm, call.this, "Resumed");
    Ok(ScriptValue::Nil)
}

#[cfg(test)]
mod tests {
    use metallic_core::Value;

    use crate::testing::{call, game, new_in, workspace};

    #[test]
    fn the_methods_set_what_the_engine_plays_and_every_sound_is_listed() {
        let mut dm = game();
        let ws = workspace(&dm);
        let sound = new_in(&mut dm, "Sound", ws, "Boom");
        assert_eq!(
            dm.state::<super::SoundList>().map(|list| list.sounds.clone()),
            Some(vec![sound])
        );
        assert_eq!(dm.get_by_name(sound, "Volume"), Ok(Value::Number(0.5)));
        call(&mut dm, sound, "Play", vec![]).unwrap();
        assert_eq!(dm.get_by_name(sound, "Playing"), Ok(Value::Bool(true)));
        assert_eq!(dm.get_by_name(sound, super::PLAY_COUNT), Ok(Value::Int(1)));
        dm.set_by_name(sound, "TimePosition", Value::Number(2.0)).unwrap();
        call(&mut dm, sound, "Pause", vec![]).unwrap();
        assert_eq!(dm.get_by_name(sound, "IsPaused"), Ok(Value::Bool(true)));
        assert_eq!(dm.get_by_name(sound, "TimePosition"), Ok(Value::Number(2.0)));
        call(&mut dm, sound, "Resume", vec![]).unwrap();
        assert_eq!(dm.get_by_name(sound, "Playing"), Ok(Value::Bool(true)));
        // Play while playing starts over.
        call(&mut dm, sound, "Play", vec![]).unwrap();
        assert_eq!(dm.get_by_name(sound, "TimePosition"), Ok(Value::Number(0.0)));
        dm.set_by_name(sound, "TimePosition", Value::Number(1.0)).unwrap();
        call(&mut dm, sound, "Stop", vec![]).unwrap();
        assert_eq!(dm.get_by_name(sound, "Playing"), Ok(Value::Bool(false)));
        assert_eq!(dm.get_by_name(sound, "TimePosition"), Ok(Value::Number(0.0)));

        // A clone is listed too.
        let copy = dm.clone_tree(sound).unwrap().unwrap();
        assert_eq!(dm.state::<super::SoundList>().map(|list| list.sounds.len()), Some(2));
        assert!(dm.state::<super::SoundList>().unwrap().sounds.contains(&copy));
    }
}
