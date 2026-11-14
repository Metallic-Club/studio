//! Who is in a session: a display name and colour each user chose, plus the id the session gave them.
//!
//! Offline Mode has no accounts. An [`Identity`] is what the user typed in Studio's profile; it proves nothing and
//! is only shown. The session's authority (the host) gives each connection a [`UserId`] of its own, never reused in
//! that session, and makes names unique by numbering repeats. Online Mode will add an account id beside these,
//! checked by the platform, without changing what the rest of the session reads.

use crate::codec::{CodecError, Reader, Writer};

/// The longest display name, in characters.
pub const MAX_NAME: usize = 32;

/// A user's id in one session, given by the host. The host is always [`UserId::HOST`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub u32);

impl UserId {
    /// The host's id.
    pub const HOST: Self = Self(1);

    /// The id after this one.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user {}", self.0)
    }
}

/// What a user calls themself and the colour others see them in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The display name: 1 to [`MAX_NAME`] characters, no control characters, trimmed.
    pub name: String,
    /// Their colour, sRGB.
    pub color: [u8; 3],
}

impl Identity {
    /// `name` (cleaned with [`clean_name`], "Guest" when nothing is left) in `color`.
    pub fn new(name: &str, color: [u8; 3]) -> Self {
        let name = clean_name(name);
        Self {
            name: if name.is_empty() { "Guest".to_owned() } else { name },
            color,
        }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.str(&self.name);
        w.u8(self.color[0]);
        w.u8(self.color[1]);
        w.u8(self.color[2]);
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        let name = r.str("a name", MAX_NAME * 4)?;
        let color = [r.u8()?, r.u8()?, r.u8()?];
        let cleaned = clean_name(name);
        if cleaned.is_empty() {
            return Err(CodecError::Invalid("an empty name"));
        }
        Ok(Self { name: cleaned, color })
    }
}

/// `name` as a display name: control characters dropped, spaces trimmed and runs of them made one, at most
/// [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in name.chars() {
        if c.is_control() {
            continue;
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        if out.chars().count() >= MAX_NAME {
            break;
        }
        out.push(c);
    }
    out
}

/// `wanted`, or `wanted (2)`, `wanted (3)` ... whichever no name in `taken` is (compared without case).
pub fn unique_name<'a>(wanted: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let free = |candidate: &str| !taken.clone().any(|name| name.eq_ignore_ascii_case(candidate));
    if free(wanted) {
        return wanted.to_owned();
    }
    (2..)
        .map(|n| {
            let suffix = format!(" ({n})");
            let room = MAX_NAME.saturating_sub(suffix.chars().count());
            let stem: String = wanted.chars().take(room).collect();
            format!("{stem}{suffix}")
        })
        .find(|candidate| free(candidate))
        .unwrap_or_else(|| wanted.to_owned())
}

/// A user in a session, as everyone sees them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    /// Their id in the session.
    pub id: UserId,
    /// Their name (unique in the session) and colour.
    pub identity: Identity,
    /// Whether they host the session.
    pub host: bool,
}

impl User {
    pub(crate) fn write(&self, w: &mut Writer) {
        w.u32(self.id.0);
        self.identity.write(w);
        w.bool(self.host);
    }

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            id: UserId(r.u32()?),
            identity: Identity::read(r)?,
            host: r.bool()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_kept_short() {
        assert_eq!(clean_name("  Ada \t  Lovelace\n"), "Ada Lovelace");
        assert_eq!(clean_name("a\u{7}b"), "ab");
        assert_eq!(clean_name(&"x".repeat(100)).chars().count(), MAX_NAME);
        assert_eq!(Identity::new("   ", [1, 2, 3]).name, "Guest");
    }

    #[test]
    fn repeated_names_are_numbered() {
        let taken = ["Ada", "ada (2)"];
        assert_eq!(unique_name("Bob", taken.iter().copied()), "Bob");
        assert_eq!(unique_name("ADA", taken.iter().copied()), "ADA (3)");
    }
}
