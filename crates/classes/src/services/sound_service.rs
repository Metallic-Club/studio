//! `SoundService` (#811): where a game's sounds can be kept, and `PlayLocalSound`, as Roblox's. Sounds play from
//! anywhere in the game (see `Sound`); this service holds them without putting them in the world.

use metallic_core::{ClassDesc, MethodCall, MethodDesc, Param, ScriptError, ScriptValue, Ty, Value};

use crate::audio::sound::{PLAY_LOCAL, start};

/// The game's sound settings and storage.
pub static CLASS: ClassDesc = ClassDesc::new("SoundService", Some("Instance"))
    .describe("Holds sounds that are not part of the world, such as music and interface sounds, and plays sounds for this player alone.")
    .service(8)
    .persisted()
    .methods(&[MethodDesc::new("PlayLocalSound", play_local_sound)
        .params(&[Param::new("sound", Ty::Class("Sound"))])
        .returns_nothing()
        .describe("Plays the sound for this player alone, wherever it is (even with no parent).")]);

fn play_local_sound(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let sound = call.args.instance(0)?;
    let class = call
        .dm
        .registry()
        .class_id(crate::audio::sound::CLASS.name)
        .ok_or_else(|| ScriptError::runtime("there is no Sound class"))?;
    if !call.dm.is_a(sound, class)? {
        return Err(ScriptError::bad_argument(1, "Sound", call.dm.class_name(sound)?));
    }
    call.dm.set_by_name(sound, PLAY_LOCAL, Value::Bool(true))?;
    start(call.dm, sound)?;
    Ok(ScriptValue::Nil)
}
