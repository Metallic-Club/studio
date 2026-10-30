//! Keyframed curves: [`Keyframe`], [`Interpolation`], tangents and [`Curve`] sampling.
//!
//! A curve is a list of keys sorted by time. Each key says how to travel to the *next* key
//! ([`Interpolation`]). Before the first key the curve holds the first value; after the last key
//! it holds the last value.
//!
//! Bezier segments use two handles per key, like a graph editor: the *out* handle of the left
//! key and the *in* handle of the right key. A handle is an offset in time (seconds, always
//! forwards for out, backwards for in) and in value. Handle times are clamped to the segment so
//! the curve stays a function of time (no loops back in time).

use crate::AnimError;
use crate::easing::{EasingDirection, EasingStyle, ease};
use crate::value::Animatable;
use serde::{Deserialize, Serialize};

/// How a key travels to the next key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Interpolation {
    /// Hold this key's value until the next key (a step). Good for sprite frames and switches.
    Constant,
    /// Straight line to the next key.
    Linear,
    /// Cubic Bezier using this key's out handle and the next key's in handle.
    #[default]
    Bezier,
    /// Roblox-style eased motion to the next key (Roblox `Pose.EasingStyle`/`EasingDirection`).
    Eased(EasingStyle, EasingDirection),
}

impl Interpolation {
    /// Short label for menus and tooltips.
    pub fn label(self) -> String {
        match self {
            Interpolation::Constant => "Constant".to_owned(),
            Interpolation::Linear => "Linear".to_owned(),
            Interpolation::Bezier => "Bezier".to_owned(),
            Interpolation::Eased(s, d) => format!("{} {}", s.label(), d.label()),
        }
    }
}

/// How a key's Bezier handles are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TangentMode {
    /// Computed from the neighbours for a smooth path, flattened at peaks and valleys so the
    /// curve never overshoots a key. The best default for beginners.
    #[default]
    AutoClamped,
    /// Computed from the neighbours (Catmull-Rom style); may overshoot at peaks.
    Auto,
    /// Set by hand; moving one handle mirrors the other's direction (keeps the curve smooth).
    Aligned,
    /// Set by hand; each handle moves on its own (a sharp corner is allowed).
    Free,
    /// Both handles horizontal: the value eases in and out of this key.
    Flat,
    /// Each handle points straight at its neighbour key.
    Linear,
}

impl TangentMode {
    /// Every mode, for menus.
    pub const ALL: [TangentMode; 6] = [
        TangentMode::AutoClamped,
        TangentMode::Auto,
        TangentMode::Aligned,
        TangentMode::Free,
        TangentMode::Flat,
        TangentMode::Linear,
    ];

    /// True when the handles are computed from neighbours rather than stored by hand.
    pub fn is_computed(self) -> bool {
        matches!(
            self,
            TangentMode::AutoClamped | TangentMode::Auto | TangentMode::Flat | TangentMode::Linear
        )
    }

    /// Short label for menus.
    pub fn label(self) -> &'static str {
        match self {
            TangentMode::AutoClamped => "Auto clamped",
            TangentMode::Auto => "Auto",
            TangentMode::Aligned => "Aligned",
            TangentMode::Free => "Free",
            TangentMode::Flat => "Flat",
            TangentMode::Linear => "Linear",
        }
    }
}

/// A Bezier handle: an offset from its key in time (seconds, `>= 0`) and in value.
///
/// For an *out* handle the point is `(key.time + time, key.value + value)`; for an *in* handle it
/// is `(key.time - time, key.value - value)`. So a straight, smooth curve through a key has equal
/// slopes `value / time` on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tangent<T> {
    /// Handle length along time, in seconds. Never negative.
    pub time: f32,
    /// Handle offset in value (a delta, see [`Animatable::delta`]).
    pub value: T,
}

impl<T: Animatable> Tangent<T> {
    /// A zero-length handle (the segment starts or ends with no pull).
    pub fn zero() -> Self {
        Self {
            time: 0.0,
            value: T::zero_delta(),
        }
    }

    /// True for a zero-length, zero-offset handle.
    pub fn is_zero(&self) -> bool {
        self.time == 0.0 && self.value == T::zero_delta()
    }
}

impl<T: Animatable> Default for Tangent<T> {
    fn default() -> Self {
        Self::zero()
    }
}

fn is_default_mode(m: &TangentMode) -> bool {
    *m == TangentMode::default()
}

fn is_default_interp(i: &Interpolation) -> bool {
    *i == Interpolation::default()
}

/// One key on a curve.
///
/// In files, handles are written only for hand-set tangent modes ([`TangentMode::Aligned`],
/// [`TangentMode::Free`]); computed handles are rebuilt on load, which keeps files short.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(bound = "T: Animatable", from = "KeyRepr<T>", into = "KeyRepr<T>")]
pub struct Keyframe<T> {
    /// Time in seconds from the start of the clip.
    pub time: f32,
    /// Value at this key.
    pub value: T,
    /// How to travel from this key to the next one.
    pub interpolation: Interpolation,
    /// How the handles are chosen.
    pub tangent_mode: TangentMode,
    /// Incoming handle (used by a Bezier segment that ends here).
    pub in_tangent: Tangent<T>,
    /// Outgoing handle (used when this key's interpolation is Bezier).
    pub out_tangent: Tangent<T>,
}

/// File form of a [`Keyframe`]: defaults and computed handles are left out.
#[derive(Serialize, Deserialize)]
#[serde(bound = "T: Animatable")]
struct KeyRepr<T> {
    time: f32,
    value: T,
    #[serde(default, skip_serializing_if = "is_default_interp")]
    interpolation: Interpolation,
    #[serde(default, skip_serializing_if = "is_default_mode")]
    tangent_mode: TangentMode,
    #[serde(default, skip_serializing_if = "Tangent::is_zero")]
    in_tangent: Tangent<T>,
    #[serde(default, skip_serializing_if = "Tangent::is_zero")]
    out_tangent: Tangent<T>,
}

impl<T: Animatable> From<KeyRepr<T>> for Keyframe<T> {
    fn from(r: KeyRepr<T>) -> Self {
        Keyframe {
            time: r.time,
            value: r.value,
            interpolation: r.interpolation,
            tangent_mode: r.tangent_mode,
            in_tangent: r.in_tangent,
            out_tangent: r.out_tangent,
        }
    }
}

impl<T: Animatable> From<Keyframe<T>> for KeyRepr<T> {
    fn from(k: Keyframe<T>) -> Self {
        let manual = !k.tangent_mode.is_computed();
        KeyRepr {
            time: k.time,
            value: k.value,
            interpolation: k.interpolation,
            tangent_mode: k.tangent_mode,
            in_tangent: if manual {
                k.in_tangent
            } else {
                Tangent::zero()
            },
            out_tangent: if manual {
                k.out_tangent
            } else {
                Tangent::zero()
            },
        }
    }
}

impl<T: Animatable> Keyframe<T> {
    /// A key with the default (Bezier, auto clamped) interpolation and zero handles.
    /// Call [`Curve::recompute_tangents`] after building a curve to fill in the handles.
    pub fn new(time: f32, value: T) -> Self {
        Self {
            time,
            value,
            interpolation: Interpolation::default(),
            tangent_mode: TangentMode::default(),
            in_tangent: Tangent::zero(),
            out_tangent: Tangent::zero(),
        }
    }

    /// The same key with another interpolation.
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// The same key with another tangent mode.
    pub fn with_tangent_mode(mut self, mode: TangentMode) -> Self {
        self.tangent_mode = mode;
        self
    }

    /// The same key with hand-set handles (switches the mode to [`TangentMode::Free`]).
    pub fn with_tangents(mut self, in_tangent: Tangent<T>, out_tangent: Tangent<T>) -> Self {
        self.in_tangent = in_tangent;
        self.out_tangent = out_tangent;
        self.tangent_mode = TangentMode::Free;
        self
    }
}

/// Keys closer than this in time are treated as the same key by [`Curve::insert`].
pub const KEY_TIME_EPSILON: f32 = 1e-4;

/// A keyframed curve of one value type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound = "T: Animatable", transparent)]
pub struct Curve<T> {
    keys: Vec<Keyframe<T>>,
}

impl<T: Animatable> Default for Curve<T> {
    fn default() -> Self {
        Self { keys: Vec::new() }
    }
}

impl<T: Animatable> Curve<T> {
    /// An empty curve.
    pub fn new() -> Self {
        Self::default()
    }

    /// A curve from keys in any order. Keys are sorted by time and computed handles are filled in.
    ///
    /// # Errors
    /// [`AnimError::InvalidKeyTime`] if a time is not finite or is negative.
    pub fn from_keys(mut keys: Vec<Keyframe<T>>) -> Result<Self, AnimError> {
        for k in &keys {
            check_time(k.time)?;
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        let mut curve = Self { keys };
        curve.recompute_tangents();
        Ok(curve)
    }

    /// Checks the invariants: finite, non-negative, strictly increasing times.
    ///
    /// # Errors
    /// [`AnimError::InvalidKeyTime`] or [`AnimError::UnsortedKeys`].
    pub fn validate(&self) -> Result<(), AnimError> {
        for k in &self.keys {
            check_time(k.time)?;
        }
        if self.keys.windows(2).any(|w| w[1].time <= w[0].time) {
            return Err(AnimError::UnsortedKeys);
        }
        Ok(())
    }

    /// The keys, sorted by time.
    pub fn keys(&self) -> &[Keyframe<T>] {
        &self.keys
    }

    /// Mutable access to one key. Changing its `time` can break the order: call [`Curve::sort`]
    /// afterwards.
    pub fn key_mut(&mut self, index: usize) -> Option<&mut Keyframe<T>> {
        self.keys.get_mut(index)
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True when the curve has no keys.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Time of the last key, or `0` when empty.
    pub fn end_time(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.time)
    }

    /// Adds a key, replacing a key at (almost) the same time. Returns its index.
    ///
    /// # Errors
    /// [`AnimError::InvalidKeyTime`] if the time is not finite or is negative.
    pub fn insert(&mut self, key: Keyframe<T>) -> Result<usize, AnimError> {
        check_time(key.time)?;
        let index = self
            .keys
            .partition_point(|k| k.time < key.time - KEY_TIME_EPSILON);
        if let Some(existing) = self.keys.get_mut(index)
            && (existing.time - key.time).abs() <= KEY_TIME_EPSILON
        {
            *existing = key;
        } else {
            self.keys.insert(index, key);
        }
        self.recompute_tangents();
        Ok(index)
    }

    /// Removes and returns the key at `index`.
    pub fn remove(&mut self, index: usize) -> Option<Keyframe<T>> {
        if index >= self.keys.len() {
            return None;
        }
        let key = self.keys.remove(index);
        self.recompute_tangents();
        Some(key)
    }

    /// Sorts keys by time after edits, merges keys that landed on the same time (the later index
    /// wins), and recomputes handles. Returns, for each old index, its new index.
    pub fn sort(&mut self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.keys.len()).collect();
        order.sort_by(|&a, &b| {
            self.keys[a]
                .time
                .total_cmp(&self.keys[b].time)
                .then(a.cmp(&b))
        });
        let mut new_keys: Vec<Keyframe<T>> = Vec::with_capacity(self.keys.len());
        let mut remap = vec![0; self.keys.len()];
        for old in order {
            let key = self.keys[old];
            match new_keys.last_mut() {
                Some(last) if (last.time - key.time).abs() <= KEY_TIME_EPSILON => *last = key,
                _ => new_keys.push(key),
            }
            remap[old] = new_keys.len() - 1;
        }
        self.keys = new_keys;
        self.recompute_tangents();
        remap
    }

    /// Fills in handles for keys whose [`TangentMode`] is computed, and clamps hand-set handle
    /// lengths to their segments. Call after moving keys.
    pub fn recompute_tangents(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let mode = self.keys[i].tangent_mode;
            let prev = if i > 0 { Some(self.keys[i - 1]) } else { None };
            let next = self.keys.get(i + 1).copied();
            let key = &mut self.keys[i];
            let in_len = prev.map_or(0.0, |p| (key.time - p.time) / 3.0);
            let out_len = next.map_or(0.0, |nx| (nx.time - key.time) / 3.0);
            if !mode.is_computed() {
                key.in_tangent.time = key.in_tangent.time.clamp(0.0, in_len * 3.0);
                key.out_tangent.time = key.out_tangent.time.clamp(0.0, out_len * 3.0);
                continue;
            }
            // Slope (value per second) on each side, as deltas.
            let (in_slope_dt, in_slope) = match mode {
                TangentMode::Flat => (1.0, T::zero_delta()),
                TangentMode::Linear => match prev {
                    Some(p) => (key.time - p.time, T::delta(key.value, p.value)),
                    None => (1.0, T::zero_delta()),
                },
                _ => auto_slope(prev, key, next),
            };
            let (out_slope_dt, out_slope) = match mode {
                TangentMode::Linear => match next {
                    Some(nx) => (nx.time - key.time, T::delta(nx.value, key.value)),
                    None => (1.0, T::zero_delta()),
                },
                _ => (in_slope_dt, in_slope),
            };
            let flat = mode == TangentMode::AutoClamped
                && match (prev, next) {
                    (Some(p), Some(nx)) => T::is_extreme(p.value, key.value, nx.value),
                    _ => true,
                };
            let (in_slope, out_slope) = if flat {
                (T::zero_delta(), T::zero_delta())
            } else {
                (in_slope, out_slope)
            };
            key.in_tangent = Tangent {
                time: in_len,
                value: T::offset(T::zero_delta(), in_slope, safe_div(in_len, in_slope_dt)),
            };
            key.out_tangent = Tangent {
                time: out_len,
                value: T::offset(T::zero_delta(), out_slope, safe_div(out_len, out_slope_dt)),
            };
        }
    }

    /// The value at `time` (seconds). `None` only when the curve has no keys.
    pub fn sample(&self, time: f32) -> Option<T> {
        let first = self.keys.first()?;
        if self.keys.len() == 1 || time <= first.time || time.is_nan() {
            return Some(first.value);
        }
        // Index of the first key strictly after `time`.
        let right = self.keys.partition_point(|k| k.time <= time);
        if right >= self.keys.len() {
            return self.keys.last().map(|k| k.value);
        }
        let a = &self.keys[right - 1];
        let b = &self.keys[right];
        Some(sample_segment(a, b, time))
    }

    /// Writes `samples` evenly spaced values between `start` and `end` into `out` (cleared first).
    /// Handy for drawing.
    pub fn sample_range(&self, start: f32, end: f32, samples: usize, out: &mut Vec<(f32, T)>) {
        out.clear();
        if samples == 0 {
            return;
        }
        let step = if samples > 1 {
            (end - start) / (samples - 1) as f32
        } else {
            0.0
        };
        for i in 0..samples {
            let t = start + step * i as f32;
            if let Some(v) = self.sample(t) {
                out.push((t, v));
            }
        }
    }
}

fn safe_div(a: f32, b: f32) -> f32 {
    if b.abs() < 1e-9 { 0.0 } else { a / b }
}

fn check_time(time: f32) -> Result<(), AnimError> {
    if time.is_finite() && time >= 0.0 {
        Ok(())
    } else {
        Err(AnimError::InvalidKeyTime(time))
    }
}

/// Catmull-Rom style slope through the neighbours: `(next - prev) / (t_next - t_prev)`.
/// Returns `(seconds, delta)` meaning a slope of `delta / seconds`.
fn auto_slope<T: Animatable>(
    prev: Option<Keyframe<T>>,
    key: &Keyframe<T>,
    next: Option<Keyframe<T>>,
) -> (f32, T) {
    match (prev, next) {
        (Some(p), Some(n)) => (n.time - p.time, T::delta(n.value, p.value)),
        (None, Some(n)) => (n.time - key.time, T::delta(n.value, key.value)),
        (Some(p), None) => (key.time - p.time, T::delta(key.value, p.value)),
        (None, None) => (1.0, T::zero_delta()),
    }
}

/// Interpolates between keys `a` and `b` at `time`, with `a.time <= time < b.time`.
pub(crate) fn sample_segment<T: Animatable>(a: &Keyframe<T>, b: &Keyframe<T>, time: f32) -> T {
    let span = b.time - a.time;
    if span <= 0.0 {
        return b.value;
    }
    let u = ((time - a.time) / span).clamp(0.0, 1.0);
    match a.interpolation {
        Interpolation::Constant => a.value,
        Interpolation::Linear => T::interpolate(a.value, b.value, u),
        Interpolation::Eased(style, dir) => T::interpolate(a.value, b.value, ease(style, dir, u)),
        Interpolation::Bezier => {
            if T::is_discrete() {
                return a.value;
            }
            // Control points in time, normalised to [0, 1] and clamped so x(s) is monotonic.
            let x1 = (a.out_tangent.time / span).clamp(0.0, 1.0);
            let x2 = 1.0 - (b.in_tangent.time / span).clamp(0.0, 1.0);
            let s = solve_bezier_x(x1, x2, u);
            let p0 = a.value;
            let p1 = T::offset(a.value, a.out_tangent.value, 1.0);
            let p2 = T::offset(b.value, b.in_tangent.value, -1.0);
            let p3 = b.value;
            T::bezier(p0, p1, p2, p3, s)
        }
    }
}

/// The 1-D cubic Bezier with control points `0, x1, x2, 1` evaluated at `s`.
pub(crate) fn bezier_1d(x1: f32, x2: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * s * x1 + 3.0 * inv * s * s * x2 + s * s * s
}

/// Finds `s` with `bezier_1d(x1, x2, s) == x` (monotonic since `x1, x2` are in `[0, 1]`).
/// Newton steps with a bisection fallback; accurate to ~1e-6.
pub(crate) fn solve_bezier_x(x1: f32, x2: f32, x: f32) -> f32 {
    let mut s = x;
    for _ in 0..8 {
        let err = bezier_1d(x1, x2, s) - x;
        if err.abs() < 1e-6 {
            return s;
        }
        let inv = 1.0 - s;
        let d = 3.0 * inv * inv * x1 + 6.0 * inv * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = x;
    for _ in 0..40 {
        let v = bezier_1d(x1, x2, s);
        if (v - x).abs() < 1e-6 {
            break;
        }
        if v < x {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    s
}
