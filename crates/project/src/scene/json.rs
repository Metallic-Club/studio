//! The layout of a scene file's JSON: one object key per line, arrays on one line.
//!
//! `serde_json`'s pretty printer puts every array element on its own line, which turns each `CFrame` into nine
//! lines and a 100,000-part scene into millions. Here arrays stay on one line (`[1.0, 2.0, 3.0]`, and the instance
//! list reads `[{ ... }, { ... }]`), while object keys get a line each, so a diff still shows which property of
//! which instance changed.

use std::io::{self, Write};

use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};

/// Writes `value` as a scene file's text: the layout above, two spaces per level, and a final newline.
pub(super) fn to_text<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let mut out = Vec::new();
    value.serialize(&mut Serializer::with_formatter(&mut out, SceneFormatter::default()))?;
    out.push(b'\n');
    // serde_json only writes UTF-8; a failure here would be its bug, reported rather than unwrapped.
    String::from_utf8(out).map_err(|error| serde::ser::Error::custom(error.to_string()))
}

/// The formatter behind [`to_text`]. Only objects indent; arrays never break a line.
#[derive(Default)]
struct SceneFormatter {
    /// How many objects are open.
    depth: usize,
    /// Whether the innermost open object has written a key, so its closing brace goes on a new line.
    has_value: bool,
}

impl SceneFormatter {
    fn newline<W: ?Sized + Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b"\n")?;
        for _ in 0..self.depth {
            writer.write_all(b"  ")?;
        }
        Ok(())
    }
}

impl Formatter for SceneFormatter {
    fn begin_array_value<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { writer.write_all(b", ") }
    }

    fn begin_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        writer.write_all(b"{")
    }

    fn end_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth = self.depth.saturating_sub(1);
        if self.has_value {
            self.newline(writer)?;
        }
        writer.write_all(b"}")
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        self.newline(writer)
    }

    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn end_object_value<W: ?Sized + Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::to_text;

    #[test]
    fn objects_indent_and_arrays_stay_on_one_line() {
        let value = json!({
            "format": 1,
            "instances": [
                {"id": 1, "properties": {"Size": [1.0, 2.0, 3.0]}},
                {"id": 2, "properties": {}},
            ],
        });
        let expected = "{\n  \"format\": 1,\n  \"instances\": [{\n    \"id\": 1,\n    \"properties\": {\n      \
                        \"Size\": [1.0, 2.0, 3.0]\n    }\n  }, {\n    \"id\": 2,\n    \"properties\": {}\n  }]\n}\n";
        assert_eq!(to_text(&value).unwrap(), expected);
    }
}
