//! `Lighting`: the sun, the sky and the light that fills the scene.
//!
//! Names and meanings follow Roblox where Roblox has the property (`ClockTime`, `Brightness`, `Ambient`,
//! `OutdoorAmbient`, `GeographicLatitude`, `GlobalShadows`, `GetSunDirection`, `GetMoonDirection`). The sky is Metallic's own: Roblox
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
    ClassDesc, DataModel, Id, MethodCall, MethodDesc, Param, PropFlags, PropMeta, ScriptError, ScriptValue, Ty,
    Value, Vector3, Widget,
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

const SECONDS_PER_DAY: i64 = 24 * 3600;

/// `clock_time` hours as `HH:MM:SS`, rounded to the nearest second (so the end of the day shows `00:00:00`, as
/// `ClockTime` wraps there).
fn format_time_of_day(clock_time: f64) -> String {
    // A float-to-int cast saturates, and `ClockTime` is finite and in [0, 24).
    let seconds = ((clock_time * 3600.0).round() as i64).rem_euclid(SECONDS_PER_DAY);
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// `text` (`H:M:S` or `H:M`, each part digits only) as hours, or why it is not a time.
fn parse_time_of_day(text: &str) -> Result<f64, ScriptError> {
    let bad = |reason: &str| {
        ScriptError::Runtime(format!(
            "TimeOfDay must look like \"14:30:00\" (hours 0-23, minutes and seconds 0-59): {reason}"
        ))
    };
    let mut parts = text.trim().split(':');
    let mut field = |name: &str, max: u32, required: bool| -> Result<u32, ScriptError> {
        let Some(part) = parts.next() else {
            return if required { Err(bad(&format!("the {name} is missing"))) } else { Ok(0) };
        };
        if part.is_empty() || part.len() > 2 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad(&format!("the {name} \"{part}\" is not a number")));
        }
        let value: u32 = part.parse().map_err(|_| bad(&format!("the {name} \"{part}\" is not a number")))?;
        if value > max {
            return Err(bad(&format!("the {name} {value} is above {max}")));
        }
        Ok(value)
    };
    let hours = field("hours", 23, true)?;
    let minutes = field("minutes", 59, true)?;
    let seconds = field("seconds", 59, false)?;
    if parts.next().is_some() {
        return Err(bad("there are too many parts"));
    }
    Ok(f64::from(hours) + f64::from(minutes) / 60.0 + f64::from(seconds) / 3600.0)
}

fn time_of_day_get(dm: &DataModel, id: Id) -> Value {
    Value::String(format_time_of_day(number(dm, id, "ClockTime", 14.0)).into())
}

fn time_of_day_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    // The data model has checked the type before it runs a setter.
    let Value::String(text) = value else {
        return Err(ScriptError::WrongType {
            expected: "string".into(),
            got: "something else".into(),
        });
    };
    let hours = parse_time_of_day(&text)?;
    dm.set_by_name(id, "ClockTime", Value::Number(hours))?;
    Ok(())
}

fn get_minutes_after_midnight(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok((number(call.dm, call.this, "ClockTime", 14.0) * 60.0).into())
}

fn set_minutes_after_midnight(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let minutes = call.args.number(0)?;
    // `ClockTime` wraps, so 1500 minutes is 01:00 the next day.
    call.dm
        .set_by_name(call.this, "ClockTime", Value::Number(minutes / 60.0))?;
    Ok(ScriptValue::Nil)
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
        .wrap(0.0, 24.0)
        .widget(Widget::Slider)
        .visual()
        .group("Sun")
        .order(0)
        .desc(
            "The time of day in hours, which places the sun: 6 is sunrise, 12 is noon, 18 is sunset. Values wrap \
             around the clock instead of stopping at the ends: 25 becomes 1 and -1 becomes 23, so a script can \
             simply add to it to run the day.",
        ),
    PropMeta::string("TimeOfDay", "14:00:00")
        .computed(time_of_day_get, Some(time_of_day_set))
        .derived_from(&["ClockTime"])
        .flags(PropFlags::NO_SAVE)
        .group("Sun")
        .order(1)
        .desc(
            "ClockTime written as hours, minutes and seconds, such as \"14:30:00\" (hours 0-23, seconds optional \
             when writing). Setting it sets ClockTime; a text that is not a time is an error.",
        ),
    PropMeta::number("Brightness", 2.0)
        // No upper limit (#481), as for the other lights (#406).
        .range(0.0, f64::INFINITY)
        .widget(Widget::Drag { speed: 0.05 })
        .visual()
        .group("Sun")
        .order(2)
        .desc(
            "How strong the sunlight is (and the moonlight, at night). 0 turns the sun off; there is no upper \
             limit.",
        ),
    PropMeta::number("GeographicLatitude", DEFAULT_LATITUDE)
        .range(-90.0, 90.0)
        .visual()
        .group("Sun")
        .order(3)
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
        .order(4)
        .desc(
            "The direction from the scene towards the sun, a unit vector set by ClockTime and GeographicLatitude. \
             It points below the horizon at night, when the moon, opposite it, lights the scene.",
        ),
    PropMeta::boolean("GlobalShadows", true)
        .visual()
        .group("Sun")
        .order(5)
        .desc(
            "Whether the sun, and the moon at night, cast shadows. Each part can still opt out with its own \
             CastShadow. Turning it off saves the cost of drawing the shadows.",
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
    MethodDesc::new("GetMinutesAfterMidnight", get_minutes_after_midnight)
        .params(&[])
        .returns(Ty::Number)
        .describe("Returns the time of day in minutes since midnight, which is ClockTime times 60."),
    MethodDesc::new("SetMinutesAfterMidnight", set_minutes_after_midnight)
        .params(&[Param::new("minutes", Ty::Number)])
        .returns_nothing()
        .describe(
            "Sets the time of day to this many minutes after midnight. Like ClockTime it wraps around the day, so \
             1500 is 01:00.",
        ),
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
    fn the_sun_s_brightness_has_no_upper_limit() {
        let mut dm = crate::new_game().unwrap();
        let lighting = lighting_of(&dm);
        for (written, stored) in [(50.0, 50.0), (1e6, 1e6), (-1.0, 0.0)] {
            dm.set_by_name(lighting, "Brightness", Value::Number(written)).unwrap();
            assert_eq!(number(&dm, lighting, "Brightness", f64::NAN), stored, "{written}");
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

    fn lighting_of(dm: &DataModel) -> Id {
        let class = dm.registry().class_id("Lighting").unwrap();
        dm.service(class).unwrap()
    }

    fn clock(dm: &DataModel, lighting: Id) -> f64 {
        number(dm, lighting, "ClockTime", f64::NAN)
    }

    fn time_of_day(dm: &DataModel, lighting: Id) -> String {
        match dm.get_by_name(lighting, "TimeOfDay") {
            Ok(Value::String(s)) => s.to_string(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn clock_time_wraps_around_the_day() {
        let mut dm = crate::new_game().unwrap();
        let lighting = lighting_of(&dm);
        for (written, read) in [(25.0, 1.0), (-1.0, 23.0), (24.0, 0.0), (48.5, 0.5), (13.25, 13.25)] {
            dm.set_by_name(lighting, "ClockTime", Value::Number(written)).unwrap();
            assert_eq!(clock(&dm, lighting), read, "{written}");
        }
    }

    #[test]
    fn time_of_day_is_clock_time_as_text() {
        let mut dm = crate::new_game().unwrap();
        let lighting = lighting_of(&dm);
        assert_eq!(time_of_day(&dm, lighting), "14:00:00");
        for (hours, text) in [
            (0.0, "00:00:00"),
            (6.5, "06:30:00"),
            (14.75, "14:45:00"),
            (23.999_99, "00:00:00"),
            (1.0 + 2.0 / 60.0 + 3.0 / 3600.0, "01:02:03"),
        ] {
            dm.set_by_name(lighting, "ClockTime", Value::Number(hours)).unwrap();
            assert_eq!(time_of_day(&dm, lighting), text, "{hours}");
        }
        // Writing the text sets ClockTime, and what it reads back is the text again.
        for text in ["00:00:00", "09:15:30", "14:30:00", "23:59:59"] {
            dm.set_by_name(lighting, "TimeOfDay", Value::String(text.into())).unwrap();
            assert_eq!(time_of_day(&dm, lighting), text);
        }
        dm.set_by_name(lighting, "TimeOfDay", Value::String("14:30".into())).unwrap();
        assert_eq!(clock(&dm, lighting), 14.5);
        dm.set_by_name(lighting, "TimeOfDay", Value::String("6:05:00".into())).unwrap();
        assert!((clock(&dm, lighting) - (6.0 + 5.0 / 60.0)).abs() < 1e-12);
    }

    #[test]
    fn a_malformed_time_of_day_is_refused_and_changes_nothing() {
        let mut dm = crate::new_game().unwrap();
        let lighting = lighting_of(&dm);
        for bad in ["", "noon", "14", "14:", ":30", "24:00:00", "14:60:00", "14:30:60", "1:2:3:4", "-1:00:00", "14:3x:00", "14.5:00"] {
            let err = dm
                .set_by_name(lighting, "TimeOfDay", Value::String(bad.into()))
                .expect_err(bad);
            assert!(err.to_string().contains("TimeOfDay must look like \"14:30:00\""), "{bad:?}: {err}");
            assert_eq!(clock(&dm, lighting), 14.0, "{bad:?}");
        }
    }

    #[test]
    fn minutes_after_midnight_round_trip_with_clock_time() {
        let mut dm = crate::new_game().unwrap();
        let lighting = lighting_of(&dm);
        let call = |dm: &mut DataModel, name: &str, args: Vec<ScriptValue>| {
            crate::testing::call(dm, lighting, name, args).unwrap()
        };
        assert_eq!(call(&mut dm, "GetMinutesAfterMidnight", vec![]), ScriptValue::Number(840.0));
        call(&mut dm, "SetMinutesAfterMidnight", vec![ScriptValue::Number(90.0)]);
        assert_eq!(clock(&dm, lighting), 1.5);
        assert_eq!(time_of_day(&dm, lighting), "01:30:00");
        assert_eq!(call(&mut dm, "GetMinutesAfterMidnight", vec![]), ScriptValue::Number(90.0));
        // It wraps like ClockTime.
        call(&mut dm, "SetMinutesAfterMidnight", vec![ScriptValue::Number(1500.0)]);
        assert_eq!(clock(&dm, lighting), 1.0);
        call(&mut dm, "SetMinutesAfterMidnight", vec![ScriptValue::Number(-60.0)]);
        assert_eq!(clock(&dm, lighting), 23.0);
        assert!(crate::testing::call(&mut dm, lighting, "SetMinutesAfterMidnight", vec![]).is_err());
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
