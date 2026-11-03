//! `Lighting`: the sun, the sky and the light that fills the scene.
//!
//! Names and meanings follow Roblox where Roblox has the property (`ClockTime`, `Brightness`, `Ambient`,
//! `OutdoorAmbient`, `GeographicLatitude`, `GetSunDirection`, `GetMoonDirection`). The sky is Metallic's own: Roblox
//! draws it from a `Sky` object's textures, Metallic from a gradient between `SkyColor` overhead and `HorizonColor`
//! at the horizon, so a game gets a sky without any assets (a textured sky can come later, on top).
//!
//! # Where the sun is
//!
//! The sun turns once a day around the world's polar axis, as seen from `GeographicLatitude` degrees north on an
//! equinox (no seasons): it rises in the east (+X) at 6, is highest in the south at 12 (+Z, for a northern
//! latitude) and sets in the west (−X) at 18. North is −Z, the way a new camera looks. `SunDirection` is that
//! direction, computed from `ClockTime` and `GeographicLatitude`, and it is what the renderer reads; the moon is
//! always opposite the sun.

use metallic_core::{
    ClassDesc, DataModel, Id, MethodCall, MethodDesc, PropFlags, PropMeta, ScriptError, ScriptValue, Ty, Value,
    Vector3, Widget,
};

/// `GeographicLatitude`'s default, as in Roblox.
pub const DEFAULT_LATITUDE: f64 = 41.733;

/// The unit vector towards the sun at `clock_time` hours (0 to 24) seen from `latitude` degrees north (see the
/// module documentation). Below the horizon its Y is negative.
pub fn sun_direction(clock_time: f64, latitude: f64) -> Vector3 {
    let hour_angle = ((clock_time - 12.0) * 15.0).to_radians();
    let latitude = latitude.to_radians();
    let (sin_h, cos_h) = hour_angle.sin_cos();
    let (sin_l, cos_l) = latitude.sin_cos();
    Vector3::new((-sin_h) as f32, (cos_l * cos_h) as f32, (sin_l * cos_h) as f32).normalize_or(Vector3::Y)
}

fn number(dm: &DataModel, id: Id, name: &str, default: f64) -> f64 {
    match dm.get_by_name(id, name) {
        Ok(Value::Number(n)) => n,
        _ => default,
    }
}

fn sun_of(dm: &DataModel, id: Id) -> Vector3 {
    sun_direction(
        number(dm, id, "ClockTime", 14.0),
        number(dm, id, "GeographicLatitude", DEFAULT_LATITUDE),
    )
}

fn sun_direction_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(sun_of(dm, id))
}

fn get_sun_direction(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(sun_of(call.dm, call.this).into())
}

fn get_moon_direction(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok((-sun_of(call.dm, call.this)).into())
}

static PROPS: &[PropMeta] = &[
    PropMeta::number("ClockTime", 14.0)
        .range(0.0, 24.0)
        .widget(Widget::Slider)
        .visual()
        .group("Sun")
        .order(0)
        .desc("The time of day in hours, which places the sun: 6 is sunrise, 12 is noon, 18 is sunset."),
    PropMeta::number("Brightness", 2.0)
        .range(0.0, 10.0)
        .visual()
        .group("Sun")
        .order(1)
        .desc("How strong the sunlight is (and the moonlight, at night). 0 turns the sun off."),
    PropMeta::number("GeographicLatitude", DEFAULT_LATITUDE)
        .range(-90.0, 90.0)
        .visual()
        .group("Sun")
        .order(2)
        .desc(
            "Where on Earth the game is, in degrees north of the equator. It sets how high the sun climbs: at 0 it \
             passes straight overhead at noon, further north it stays lower in the south.",
        ),
    PropMeta::vector3("SunDirection", 0.0, 1.0, 0.0)
        .computed(sun_direction_get, None)
        .derived_from(&["ClockTime", "GeographicLatitude"])
        .read_only()
        .flags(PropFlags::NO_SAVE)
        .group("Sun")
        .order(3)
        .desc(
            "The direction from the scene towards the sun, a unit vector set by ClockTime and GeographicLatitude. \
             It points below the horizon at night, when the moon, opposite it, lights the scene.",
        ),
    PropMeta::color3("SkyColor", 0.53, 0.72, 0.95)
        .visual()
        .group("Sky")
        .order(0)
        .desc("The colour of the sky overhead in the middle of the day. It also tints the light from the sky."),
    PropMeta::color3("HorizonColor", 0.8, 0.88, 0.96)
        .visual()
        .group("Sky")
        .order(1)
        .desc("The colour of the sky at the horizon in the middle of the day; the sky fades to it from SkyColor."),
    PropMeta::color3("OutdoorAmbient", 0.5, 0.5, 0.5)
        .visual()
        .group("Ambient")
        .order(0)
        .desc(
            "The light the sky gives outdoors, from above, filling what the sun does not reach. It dims at night \
             and is bluer then.",
        ),
    PropMeta::color3("Ambient", 0.0, 0.0, 0.0)
        .visual()
        .group("Ambient")
        .order(1)
        .desc(
            "Light added evenly everywhere, indoors and out, at every time of day: raise it to keep dark places and \
             nights brighter.",
        ),
];

/// The direction the sun and moon methods return.
const VECTOR3: Ty = Ty::Vector3;

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("GetSunDirection", get_sun_direction)
        .params(&[])
        .returns(VECTOR3)
        .describe("Returns the unit vector from the scene towards the sun, the same as SunDirection."),
    MethodDesc::new("GetMoonDirection", get_moon_direction)
        .params(&[])
        .returns(VECTOR3)
        .describe("Returns the unit vector from the scene towards the moon, which is always opposite the sun."),
];

/// The service that controls how the scene is lit.
pub static CLASS: ClassDesc = ClassDesc::new("Lighting", Some("Instance"))
    .describe(
        "Controls how the scene is lit: the time of day, the sun and moon, the colours of the sky and the ambient \
         light.",
    )
    .service(1)
    .persisted()
    .props(PROPS)
    .methods(METHODS);

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vector3, b: Vector3) -> bool {
        (a - b).abs().max_element() < 1e-5
    }

    #[test]
    fn the_sun_rises_in_the_east_and_sets_in_the_west() {
        // At an equinox the sun is on the horizon at 6 and 18, wherever you are.
        assert!(close(sun_direction(6.0, DEFAULT_LATITUDE), Vector3::X));
        assert!(close(sun_direction(18.0, DEFAULT_LATITUDE), Vector3::NEG_X));
        // At noon it is in the south (+Z), as high as 90° minus the latitude.
        let noon = sun_direction(12.0, DEFAULT_LATITUDE);
        assert!(noon.x.abs() < 1e-6 && noon.z > 0.0);
        let elevation = f64::from(noon.y).asin().to_degrees();
        assert!((elevation - (90.0 - DEFAULT_LATITUDE)).abs() < 1e-3, "{elevation}");
        // On the equator straight overhead; at midnight straight below.
        assert!(close(sun_direction(12.0, 0.0), Vector3::Y));
        assert!(close(sun_direction(0.0, 0.0), Vector3::NEG_Y));
        assert!(close(sun_direction(24.0, 0.0), Vector3::NEG_Y));
        for hour in 0..=24 {
            assert!((sun_direction(f64::from(hour), 30.0).length() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn sun_direction_follows_clock_time_and_latitude() {
        let mut dm = crate::new_game().unwrap();
        let class = dm.registry().class_id("Lighting").unwrap();
        let lighting = dm.service(class).unwrap();
        let read = |dm: &DataModel| match dm.get_by_name(lighting, "SunDirection") {
            Ok(Value::Vector3(v)) => v,
            other => panic!("{other:?}"),
        };
        assert!(close(read(&dm), sun_direction(14.0, DEFAULT_LATITUDE)));
        dm.set_by_name(lighting, "ClockTime", Value::Number(6.0)).unwrap();
        assert!(close(read(&dm), Vector3::X));
        dm.set_by_name(lighting, "GeographicLatitude", Value::Number(0.0)).unwrap();
        dm.set_by_name(lighting, "ClockTime", Value::Number(12.0)).unwrap();
        assert!(close(read(&dm), Vector3::Y));
        // Computed, so it cannot be written.
        assert!(
            dm.set_by_name(lighting, "SunDirection", Value::Vector3(Vector3::X)).is_err(),
            "SunDirection is read-only"
        );
    }

    #[test]
    fn the_moon_is_opposite_the_sun() {
        let mut dm = crate::new_game().unwrap();
        let class = dm.registry().class_id("Lighting").unwrap();
        let lighting = dm.service(class).unwrap();
        dm.set_by_name(lighting, "ClockTime", Value::Number(22.0)).unwrap();
        let mut direction = |name: &str| match crate::testing::call(&mut dm, lighting, name, Vec::new()) {
            Ok(ScriptValue::Vector3(v)) => v,
            other => panic!("{other:?}"),
        };
        let (sun, moon) = (direction("GetSunDirection"), direction("GetMoonDirection"));
        assert!(close(sun, sun_direction(22.0, DEFAULT_LATITUDE)));
        assert!(sun.y < 0.0 && close(moon, -sun));
    }
}
