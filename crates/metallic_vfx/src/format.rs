//! The `.vfx.ron` text format: load, save and validate effects.
//!
//! Files are RON, written with stable field order, two-space indentation and defaults left out, so an
//! edit in the editor changes as few lines as possible in a diff. Colours are `"#RRGGBB"`, vectors
//! `(x, y, z)`, ranges `(min, max)`. Colours are stored with 8 bits per channel.

use crate::VfxError;
use crate::effect::{
    Effect, Emitter, FORMAT_VERSION, InitModule, MAX_CAPACITY, Shape, SpawnModule, UpdateModule,
};

impl Effect {
    /// Writes the effect as pretty RON.
    ///
    /// # Errors
    /// [`VfxError::Format`] if serialisation fails (it does not for valid effects).
    pub fn to_ron(&self) -> Result<String, VfxError> {
        let config = ron::ser::PrettyConfig::new()
            .indentor("  ".to_string())
            .new_line("
".to_string())
            .struct_names(false)
            .separate_tuple_members(false)
            .compact_arrays(false);
        let mut text = ron::ser::to_string_pretty(self, config)
            .map_err(|e| VfxError::Format(e.to_string()))?;
        text.push('\n');
        Ok(text)
    }

    /// Parses an effect file and validates it.
    ///
    /// # Errors
    /// [`VfxError::Parse`] for malformed text, [`VfxError::UnsupportedVersion`] for files from a newer
    /// format, [`VfxError::Invalid`] when a value is out of range.
    pub fn from_ron(text: &str) -> Result<Self, VfxError> {
        let effect: Effect = ron::from_str(text).map_err(|e| VfxError::Parse(e.to_string()))?;
        if effect.format > FORMAT_VERSION {
            return Err(VfxError::UnsupportedVersion(effect.format));
        }
        effect.validate()?;
        Ok(effect)
    }

    /// Checks every number is finite and every count is in range.
    ///
    /// # Errors
    /// [`VfxError::Invalid`] naming the first bad field.
    pub fn validate(&self) -> Result<(), VfxError> {
        let bad = |path: String, reason: &str| {
            Err(VfxError::Invalid {
                path,
                reason: reason.to_string(),
            })
        };
        if !self.duration.is_finite() || self.duration < 0.0 {
            return bad("duration".into(), "must be a finite number >= 0");
        }
        for (i, e) in self.emitters.iter().enumerate() {
            let at = format!("emitters[{i}] `{}`", e.name);
            validate_emitter(e, &at)?;
        }
        for (i, t) in self.trails.iter().enumerate() {
            let at = format!("trails[{i}] `{}`", t.name);
            if !(t.lifetime.is_finite() && t.lifetime > 0.0) {
                return bad(format!("{at}.lifetime"), "must be > 0");
            }
            if !(t.attachment0.is_finite() && t.attachment1.is_finite()) {
                return bad(format!("{at}.attachment"), "must be finite");
            }
            if !(t.width_scale.is_finite() && t.transparency.is_finite()) {
                return bad(format!("{at}"), "curves must be finite");
            }
        }
        for (i, b) in self.beams.iter().enumerate() {
            let at = format!("beams[{i}] `{}`", b.name);
            if b.segments == 0 || b.segments > 1000 {
                return bad(format!("{at}.segments"), "must be 1..=1000");
            }
            let nums = [
                b.width0,
                b.width1,
                b.curve_size0,
                b.curve_size1,
                b.texture_length,
                b.texture_speed,
                b.jitter,
                b.jitter_fps,
                b.brightness,
            ];
            if !nums.iter().all(|v| v.is_finite())
                || !(b.attachment0.is_finite() && b.attachment1.is_finite())
            {
                return bad(at, "numbers must be finite");
            }
        }
        Ok(())
    }
}

fn validate_emitter(e: &Emitter, at: &str) -> Result<(), VfxError> {
    let bad = |field: &str, reason: &str| {
        Err(VfxError::Invalid {
            path: format!("{at}.{field}"),
            reason: reason.to_string(),
        })
    };
    if e.capacity == 0 || e.capacity > MAX_CAPACITY {
        return bad("capacity", "must be 1..=262144");
    }
    if !(e.time_scale.is_finite() && e.time_scale >= 0.0) {
        return bad("time_scale", "must be >= 0");
    }
    if !(e.delay.is_finite() && e.delay >= 0.0) {
        return bad("delay", "must be >= 0");
    }
    if !(e.fixed_fps.is_finite() && e.fixed_fps >= 0.0) {
        return bad("fixed_fps", "must be >= 0");
    }
    for m in &e.spawn {
        let ok = match m {
            SpawnModule::Rate { per_second, .. } => per_second.is_finite() && *per_second >= 0.0,
            SpawnModule::Burst { time, interval, .. } => {
                time.is_finite() && *time >= 0.0 && interval.is_finite() && *interval >= 0.0
            }
            SpawnModule::Distance { per_stud, .. } => per_stud.is_finite() && *per_stud >= 0.0,
        };
        if !ok {
            return bad(m.label(), "values must be finite and >= 0");
        }
    }
    for m in &e.init {
        let ok = match m {
            InitModule::Shape { shape, .. } => match shape {
                Shape::Point | Shape::MeshSurface { .. } => true,
                Shape::Sphere { radius, partial } => radius.is_finite() && partial.is_finite(),
                Shape::Box { size } => size.is_finite(),
                Shape::Disc { radius, inner } => radius.is_finite() && inner.is_finite(),
                Shape::Cylinder {
                    radius,
                    height,
                    top,
                } => radius.is_finite() && height.is_finite() && top.is_finite(),
                Shape::Cone { radius, angle } => radius.is_finite() && angle.is_finite(),
                Shape::Edge { a, b } => a.is_finite() && b.is_finite(),
            },
            InitModule::Lifetime { seconds, .. } => seconds.is_finite(),
            InitModule::Velocity { speed, spread, .. } => {
                speed.is_finite() && spread.0.is_finite() && spread.1.is_finite()
            }
            InitModule::Size { size, .. } => size.is_finite(),
            InitModule::Color { .. } => true,
            InitModule::Rotation { angle, spin, .. } => angle.is_finite() && spin.is_finite(),
            InitModule::InheritVelocity { amount, .. } => amount.is_finite(),
        };
        if !ok {
            return bad(m.label(), "values must be finite");
        }
    }
    for m in &e.update {
        let ok = match m {
            UpdateModule::Acceleration { accel, .. } => accel.is_finite(),
            UpdateModule::Drag { drag, .. } => drag.is_finite(),
            UpdateModule::CurlNoise {
                strength,
                scale,
                scroll,
                ..
            } => strength.is_finite() && scale.is_finite() && *scale > 0.0 && scroll.is_finite(),
            UpdateModule::Vortex {
                axis,
                center,
                swirl,
                pull,
                ..
            } => axis.is_finite() && center.is_finite() && swirl.is_finite() && pull.is_finite(),
            UpdateModule::Attractor {
                position,
                strength,
                radius,
                kill_radius,
                ..
            } => {
                position.is_finite()
                    && strength.is_finite()
                    && radius.is_finite()
                    && *radius > 0.0
                    && kill_radius.is_finite()
            }
            UpdateModule::SpeedLimit { max, .. } => max.is_finite() && *max >= 0.0,
            UpdateModule::CollisionPlane {
                point,
                normal,
                bounce,
                friction,
                ..
            } => {
                point.is_finite()
                    && normal.is_finite()
                    && normal.length() > 1e-6
                    && bounce.is_finite()
                    && friction.is_finite()
            }
            UpdateModule::SizeOverLife { curve, .. }
            | UpdateModule::TransparencyOverLife { curve, .. }
            | UpdateModule::SquashOverLife { curve, .. }
            | UpdateModule::ErosionOverLife { curve, .. } => curve.is_finite(),
            UpdateModule::ColorOverLife { .. } => true,
        };
        if !ok {
            return bad(m.label(), "values must be finite and in range");
        }
    }
    let r = &e.render;
    if !(r.light_emission.is_finite()
        && r.brightness.is_finite()
        && r.z_offset.is_finite()
        && r.pixel_snap.is_finite()
        && r.pixel_snap >= 0.0)
    {
        return bad("render", "numbers must be finite");
    }
    if let Some(fb) = &r.flipbook
        && (fb.columns == 0 || fb.rows == 0 || fb.columns > 64 || fb.rows > 64 || !fb.fps.is_finite())
    {
        return bad("render.flipbook", "grid must be 1..=64 and fps finite");
    }
    Ok(())
}
