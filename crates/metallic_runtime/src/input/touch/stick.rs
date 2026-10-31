//! The touch thumbstick (spec 7.1): base radius R, knob travel R, output = offset / R with y up.
//!
//! The dead zone (0.15 R) is applied by the `TouchStick` binding, with the same radial rescale as
//! a gamepad stick, so the output starts from 0 at the edge of the dead zone.

/// How the stick base is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StickMode {
    /// The base appears where the thumb lands in the stick zone and follows a finger that goes
    /// past R (the default).
    #[default]
    Floating,
    /// The base stays at its rest position; the knob stops at R.
    Fixed,
}

/// One held stick, in dp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stick {
    /// Where the base is now.
    pub base: [f32; 2],
    /// Where the finger is.
    pub finger: [f32; 2],
    /// Base radius R.
    pub radius: f32,
    /// Floating or fixed.
    pub mode: StickMode,
}

impl Stick {
    /// A finger lands at `at`. A floating base appears under it; a fixed base stays at `rest`.
    pub fn begin(mode: StickMode, rest: [f32; 2], radius: f32, at: [f32; 2]) -> Stick {
        let base = match mode {
            StickMode::Floating => at,
            StickMode::Fixed => rest,
        };
        let mut s = Stick {
            base,
            finger: at,
            radius: radius.max(1.0),
            mode,
        };
        s.drag(at);
        s
    }

    /// The finger moves to `at`. Past R a floating base follows so the knob stays at R.
    pub fn drag(&mut self, at: [f32; 2]) {
        self.finger = at;
        if self.mode == StickMode::Floating {
            let d = [at[0] - self.base[0], at[1] - self.base[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if len > self.radius {
                let k = (len - self.radius) / len;
                self.base = [self.base[0] + d[0] * k, self.base[1] + d[1] * k];
            }
        }
    }

    /// The finger's offset from the base, limited to R.
    pub fn offset(&self) -> [f32; 2] {
        let d = [self.finger[0] - self.base[0], self.finger[1] - self.base[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len > self.radius && len > 0.0 {
            [d[0] * self.radius / len, d[1] * self.radius / len]
        } else {
            d
        }
    }

    /// Where the knob is drawn.
    pub fn knob(&self) -> [f32; 2] {
        let o = self.offset();
        [self.base[0] + o[0], self.base[1] + o[1]]
    }

    /// The raw stick value sent as `TouchStick`: offset / R, length at most 1, x right, y up.
    pub fn value(&self) -> [f32; 2] {
        let o = self.offset();
        let v = [o[0] / self.radius, -o[1] / self.radius];
        if v[0].is_finite() && v[1].is_finite() {
            v
        } else {
            [0.0; 2]
        }
    }
}
