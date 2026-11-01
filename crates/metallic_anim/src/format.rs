//! Text file format for clips, skeletons and state machines (RON, diff-friendly).
//!
//! Files follow the project's "files are text" rule: one clip per file, a header with a format
//! name and version, then the data. Each keyframe is written on its own line, so moving one key
//! changes one line in a diff. Default fields (Bezier interpolation, auto-clamped tangents, zero
//! handles) are left out to keep files short and readable.
//!
//! ```text
//! (
//!     format: "metallic-clip",
//!     version: 1,
//!     clip: (
//!         name: "Wave",
//!         duration: 1.0,
//!         ...
//!                 (time: 0.5, value: 0.8, interpolation: Linear),
//! ```

use crate::AnimError;
use crate::clip::Clip;
use crate::skeleton::Skeleton;
use crate::state_machine::StateMachine;
use serde::{Deserialize, Serialize};

/// Current file version written by this crate.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize)]
struct Out<'a, T: Serialize> {
    format: &'a str,
    version: u32,
    data: &'a T,
}

#[derive(Deserialize)]
struct In<T> {
    format: String,
    version: u32,
    data: T,
}

/// Depth at which RON output switches to one line per value: deep enough that tracks are laid
/// out on separate lines, shallow enough that every keyframe sits on a single line.
const PRETTY_DEPTH: usize = 5;

fn write<T: Serialize>(kind: &str, data: &T) -> Result<String, AnimError> {
    let config = ron::ser::PrettyConfig::new()
        .depth_limit(PRETTY_DEPTH)
        .indentor("    ".to_owned())
        .new_line(
            "
"
            .to_owned(),
        )
        .separate_tuple_members(false)
        .struct_names(false);
    let mut text = ron::ser::to_string_pretty(
        &Out {
            format: kind,
            version: FORMAT_VERSION,
            data,
        },
        config,
    )
    .map_err(|e| AnimError::Format(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

fn read<T: for<'de> Deserialize<'de>>(kind: &str, text: &str) -> Result<T, AnimError> {
    let file: In<T> = ron::from_str(text).map_err(|e| AnimError::Format(e.to_string()))?;
    if file.format != kind {
        return Err(AnimError::Format(format!(
            "expected a `{kind}` file, found `{}`",
            file.format
        )));
    }
    if file.version == 0 || file.version > FORMAT_VERSION {
        return Err(AnimError::UnsupportedVersion(file.version));
    }
    Ok(file.data)
}

/// Writes a clip as text.
///
/// # Errors
/// [`AnimError::Format`] if serialisation fails (it should not for valid clips).
pub fn clip_to_text(clip: &Clip) -> Result<String, AnimError> {
    write("metallic-clip", clip)
}

/// Reads and validates a clip.
///
/// # Errors
/// [`AnimError::Format`] on syntax errors or the wrong file kind,
/// [`AnimError::UnsupportedVersion`] for files from a newer Metallic, and clip validation errors.
pub fn clip_from_text(text: &str) -> Result<Clip, AnimError> {
    let mut clip: Clip = read("metallic-clip", text)?;
    clip.validate()?;
    // Handles of computed tangent modes are not trusted from disk.
    for t in &mut clip.tracks {
        crate::with_curve!(&mut t.channel, c => c.recompute_tangents());
    }
    Ok(clip)
}

/// Writes a skeleton as text.
///
/// # Errors
/// [`AnimError::Format`] if serialisation fails.
pub fn skeleton_to_text(skeleton: &Skeleton) -> Result<String, AnimError> {
    write("metallic-skeleton", skeleton)
}

/// Reads a skeleton.
///
/// # Errors
/// [`AnimError::Format`], [`AnimError::UnsupportedVersion`] or [`AnimError::InvalidSkeleton`].
pub fn skeleton_from_text(text: &str) -> Result<Skeleton, AnimError> {
    read("metallic-skeleton", text)
}

/// Writes a state machine as text.
///
/// # Errors
/// [`AnimError::Format`] if serialisation fails.
pub fn state_machine_to_text(machine: &StateMachine) -> Result<String, AnimError> {
    write("metallic-state-machine", machine)
}

/// Reads a state machine (names are checked later by
/// [`StateMachinePlayer::new`](crate::StateMachinePlayer::new)).
///
/// # Errors
/// [`AnimError::Format`] or [`AnimError::UnsupportedVersion`].
pub fn state_machine_from_text(text: &str) -> Result<StateMachine, AnimError> {
    read("metallic-state-machine", text)
}
