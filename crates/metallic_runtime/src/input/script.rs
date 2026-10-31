//! Scripted input for headless runs and tests: `metallic-player --input-script <file>`.
//!
//! One event per line: `frame device control state [values]`. Blank lines and lines starting with
//! `#` are skipped. Devices: `keyboard` (also `mouse`), `gamepad` or `gamepad0` to `gamepad3`,
//! `touch`. States:
//!
//! | state | meaning |
//! |---|---|
//! | `down [v]` | button pressed (value 1, or `v` for an analog trigger) |
//! | `up` | button released |
//! | `axis a [b]` | stick level (`a b`) or button level (`a`) |
//! | `pos x y` | pointer position |
//! | `delta a [b]` | mouse movement (`a b`) or wheel notches (`a`) |
//!
//! Fingers on the touch screen use `touch0` to `touch9` (the finger), a phase `Down`, `Move`,
//! `Up` or `Cancel` in place of the control, and `pos x y` in dp from the top-left of the screen.
//! They go to the touch overlay ([`TouchControls`](super::touch::TouchControls)), which decides
//! what the finger presses, through [`poll_fingers`](ScriptedSource::poll_fingers).
//!
//! ```text
//! 30  keyboard  Space       down
//! 36  keyboard  Space       up
//! 40  gamepad0  Thumbstick1 axis  0.0 1.0
//! 90  touch0    Down        pos   140 300
//! ```
use super::control::{Control, ControlKind, Device, RawEvent, RawSource};
use super::touch::TouchPhase;

/// One finger event of a script: finger `finger` did `phase` at `pos` dp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FingerEvent {
    /// The finger, 0 to 9.
    pub finger: u64,
    /// What it did.
    pub phase: TouchPhase,
    /// Where, in dp from the top-left of the screen.
    pub pos: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FingerLine {
    frame: u64,
    event: FingerEvent,
}

fn finger_line(words: &[&str]) -> Option<(u64, TouchPhase)> {
    let n = words[1].to_ascii_lowercase();
    let finger: u64 = n.strip_prefix("touch")?.parse().ok()?;
    let phase = match words[2] {
        "Down" => TouchPhase::Down,
        "Move" => TouchPhase::Move,
        "Up" => TouchPhase::Up,
        "Cancel" => TouchPhase::Cancel,
        _ => return None,
    };
    (finger < 10 && words[3] == "pos").then_some((finger, phase))
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Line {
    frame: u64,
    device: Device,
    control: Control,
    value: f32,
    pos: [f32; 2],
}

/// A [`RawSource`] that replays the lines of an input script, one frame per `poll`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedSource {
    lines: Vec<Line>,
    next: usize,
    fingers: Vec<FingerLine>,
    next_finger: usize,
    frame: u64,
    frame_dt: f64,
}

fn device(s: &str) -> Option<Device> {
    let s = s.to_ascii_lowercase();
    match s.as_str() {
        "keyboard" | "mouse" | "keyboardmouse" => Some(Device::KeyboardMouse),
        "touch" | "touch0" => Some(Device::Touch),
        "gamepad" => Some(Device::Gamepad(0)),
        _ => {
            let n: u8 = s.strip_prefix("gamepad")?.parse().ok()?;
            (n < 4).then_some(Device::Gamepad(n))
        }
    }
}

impl ScriptedSource {
    /// Parses a script. Event times are `frame * frame_dt` seconds.
    pub fn parse(text: &str, frame_dt: f64) -> Result<Self, String> {
        let mut lines = Vec::new();
        let mut fingers = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |msg: String| format!("line {}: {msg}", n + 1);
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() < 4 {
                return Err(at("want `frame device control state [values]`".into()));
            }
            let frame: u64 = words[0]
                .parse()
                .map_err(|_| at(format!("bad frame `{}`", words[0])))?;
            if let Some((finger, phase)) = finger_line(&words) {
                let nums: Vec<f32> = words[4..].iter().filter_map(|w| w.parse().ok()).collect();
                let ok = nums.len() == 2 && words.len() == 6 && nums.iter().all(|v| v.is_finite());
                if !ok {
                    return Err(at("a finger wants `pos x y`".into()));
                }
                fingers.push(FingerLine {
                    frame,
                    event: FingerEvent {
                        finger,
                        phase,
                        pos: [nums[0], nums[1]],
                    },
                });
                continue;
            }
            let dev =
                device(words[1]).ok_or_else(|| at(format!("unknown device `{}`", words[1])))?;
            let control = Control::from_name(words[2])
                .ok_or_else(|| at(format!("unknown control `{}`", words[2])))?;
            let mut nums = Vec::new();
            for w in &words[4..] {
                let v: f32 = w.parse().map_err(|_| at(format!("bad number `{w}`")))?;
                if !v.is_finite() {
                    return Err(at(format!("bad number `{w}`")));
                }
                nums.push(v);
            }
            let num = |i: usize, default: f32| nums.get(i).copied().unwrap_or(default);
            let need = |count: usize| {
                if nums.len() >= count {
                    Ok(())
                } else {
                    Err(at(format!("`{}` needs {count} number(s)", words[3])))
                }
            };
            let (value, pos) = match words[3] {
                "down" => (num(0, 1.0), [0.0; 2]),
                "up" => (0.0, [0.0; 2]),
                "axis" | "delta" => {
                    need(1)?;
                    (num(0, 0.0), [num(0, 0.0), num(1, 0.0)])
                }
                "pos" => {
                    need(2)?;
                    (0.0, [num(0, 0.0), num(1, 0.0)])
                }
                other => return Err(at(format!("unknown state `{other}`"))),
            };
            lines.push(Line {
                frame,
                device: dev,
                control,
                value,
                pos,
            });
        }
        lines.sort_by_key(|l| l.frame);
        fingers.sort_by_key(|l| l.frame);
        Ok(Self {
            lines,
            next: 0,
            fingers,
            next_finger: 0,
            frame: 0,
            frame_dt,
        })
    }

    /// Frame of the last line (0 for an empty script).
    pub fn last_frame(&self) -> u64 {
        let raw = self.lines.last().map_or(0, |l| l.frame);
        raw.max(self.fingers.last().map_or(0, |l| l.frame))
    }

    /// True when the script has finger lines (it needs the touch overlay).
    pub fn has_fingers(&self) -> bool {
        !self.fingers.is_empty()
    }

    /// Appends every finger event up to and including `frame` not emitted yet, in file order.
    pub fn poll_fingers(&mut self, frame: u64, out: &mut Vec<FingerEvent>) {
        while let Some(l) = self
            .fingers
            .get(self.next_finger)
            .filter(|l| l.frame <= frame)
        {
            out.push(l.event);
            self.next_finger += 1;
        }
    }

    /// Appends every event up to and including `frame` that has not been emitted yet, stamped
    /// with `time`. For callers that keep their own clock.
    pub fn poll_frame(&mut self, frame: u64, time: f64, out: &mut Vec<RawEvent>) {
        while let Some(l) = self.lines.get(self.next).filter(|l| l.frame <= frame) {
            out.push(match l.control.kind() {
                ControlKind::Button | ControlKind::Wheel => {
                    RawEvent::button(l.device, l.control, l.value, time)
                }
                _ => RawEvent::vector(l.device, l.control, l.pos, time),
            });
            self.next += 1;
        }
    }
}

impl RawSource for ScriptedSource {
    /// Emits the events of the next frame, stamped `frame * frame_dt`.
    fn poll(&mut self, out: &mut Vec<RawEvent>) {
        let time = self.frame as f64 * self.frame_dt;
        self.poll_frame(self.frame, time, out);
        self.frame += 1;
    }
}
