//! Luau's colours (#529): which [`Kind`] each token of a script is, and the layout that paints them.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

use super::lexer::{Token, TokenKind, lex};
use super::settings::{CodeEditor, Kind};

/// The globals Luau and Metallic give every script, coloured as [`Kind::Builtin`].
pub const BUILTINS: &[&str] = &[
    // Metallic's.
    "game",
    "workspace",
    "script",
    "Instance",
    "Enum",
    "Vector3",
    "Vector2",
    "CFrame",
    "Color3",
    "task",
    "typeof",
    "require",
    "tick",
    "time",
    "wait",
    "delay",
    "spawn",
    "warn",
    "UDim",
    "UDim2",
    "BrickColor",
    "Ray",
    // Luau's.
    "print",
    "error",
    "assert",
    "pcall",
    "xpcall",
    "select",
    "type",
    "tostring",
    "tonumber",
    "pairs",
    "ipairs",
    "next",
    "setmetatable",
    "getmetatable",
    "rawget",
    "rawset",
    "rawequal",
    "rawlen",
    "unpack",
    "math",
    "string",
    "table",
    "coroutine",
    "os",
    "utf8",
    "bit32",
    "buffer",
    "debug",
    "self",
    "_G",
    "_VERSION",
];

/// The kind of each token of `source`, by its place: a name after `.` or `:` is a member, a name called or after
/// `function` is a function, and a global of [`BUILTINS`] is built in (unless it is a member).
pub fn kinds(source: &str, tokens: &[Token]) -> Vec<Kind> {
    let mut out = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        let before = index.checked_sub(1).map(|at| &tokens[at]);
        let after = tokens.get(index + 1);
        let kind = match token.kind {
            TokenKind::Keyword if token.is_constant(source) => Kind::Constant,
            TokenKind::Keyword => Kind::Keyword,
            TokenKind::Number => Kind::Number,
            TokenKind::String => Kind::String,
            TokenKind::Comment => Kind::Comment,
            TokenKind::Symbol => Kind::Operator,
            TokenKind::Unknown => Kind::Text,
            TokenKind::Name => {
                let member = before.is_some_and(|before| before.is(source, ".") || before.is(source, ":"));
                // `function f`, and `function a.b` or `function a:b`: the name after the function's table.
                let table = after.is_some_and(|after| after.is(source, ".") || after.is(source, ":"));
                let declared = (!table && before.is_some_and(|before| before.is(source, "function")))
                    || (member && !table && function_name_before(source, tokens, index));
                let called = after.is_some_and(|after| {
                    after.is(source, "(")
                        || (after.kind == TokenKind::String && !after.text(source).starts_with('`'))
                        || after.is(source, "{")
                });
                if declared || called {
                    Kind::Function
                } else if member {
                    Kind::Member
                } else if BUILTINS.contains(&token.text(source)) {
                    Kind::Builtin
                } else {
                    Kind::Text
                }
            }
        };
        out.push(kind);
    }
    out
}

/// Whether the name at `index` ends a function's name: `function a.b.c` or `function a:c`.
fn function_name_before(source: &str, tokens: &[Token], index: usize) -> bool {
    // Back over `name (. name)*` to see `function`.
    let mut at = index;
    loop {
        let Some(previous) = at.checked_sub(1) else {
            return false;
        };
        let separator = &tokens[previous];
        if !(separator.is(source, ".") || separator.is(source, ":")) {
            return separator.is(source, "function");
        }
        let Some(name) = previous.checked_sub(1) else {
            return false;
        };
        if tokens[name].kind != TokenKind::Name {
            return false;
        }
        at = name;
        if at == 0 {
            return false;
        }
        if tokens[at - 1].is(source, "function") {
            return true;
        }
    }
}

/// The colour of each byte range of `source`, in order and covering it: [`Kind::Text`] between tokens.
pub fn spans(source: &str) -> Vec<(std::ops::Range<usize>, Kind)> {
    let tokens = lex(source);
    let kinds = kinds(source, &tokens);
    let mut out = Vec::with_capacity(tokens.len() * 2);
    let mut at = 0;
    for (token, kind) in tokens.iter().zip(kinds) {
        if token.start > at {
            out.push((at..token.start, Kind::Text));
        }
        out.push((token.start..token.end, kind));
        at = token.end;
    }
    if at < source.len() {
        out.push((at..source.len(), Kind::Text));
    }
    out
}

/// The layout of `source` in `settings`' font and the colours of the theme (`dark` or light), not wrapped, with room
/// for a swatch before each colour written ([`super::swatch`], #545).
pub fn layout_job(source: &str, settings: &CodeEditor, font: &FontId, dark: bool) -> LayoutJob {
    let palette = settings.palette(dark);
    let format = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        line_height: Some(font.size * settings.line_spacing),
        extra_letter_spacing: settings.letter_spacing,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    let swatches: Vec<usize> = super::swatch::find(source).iter().map(|swatch| swatch.start).collect();
    let room = super::view::swatch_room(settings);
    for (range, kind) in spans(source) {
        let leading = if swatches.contains(&range.start) { room } else { 0.0 };
        job.append(&source[range], leading, format(palette.color(kind)));
    }
    if source.is_empty() {
        job.append("", 0.0, format(palette.color(Kind::Text)));
    }
    // Code is not wrapped: a long line scrolls.
    job.wrap.max_width = f32::INFINITY;
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(source: &str) -> Vec<(&str, Kind)> {
        spans(source)
            .into_iter()
            .filter(|(range, _)| !source[range.clone()].trim().is_empty())
            .map(|(range, kind)| (&source[range], kind))
            .collect()
    }

    #[test]
    fn each_token_has_its_kind_by_its_place() {
        use Kind::*;
        let source = "local part = workspace.Baseplate:Clone()\nfunction M.open(door) print(\"hi\", nil) end -- done";
        assert_eq!(
            named(source),
            [
                ("local", Keyword),
                ("part", Text),
                ("=", Operator),
                ("workspace", Builtin),
                (".", Operator),
                ("Baseplate", Member),
                (":", Operator),
                ("Clone", Function),
                ("(", Operator),
                (")", Operator),
                ("function", Keyword),
                ("M", Text),
                (".", Operator),
                ("open", Function),
                ("(", Operator),
                ("door", Text),
                (")", Operator),
                ("print", Function),
                ("(", Operator),
                ("\"hi\"", String),
                (",", Operator),
                ("nil", Constant),
                (")", Operator),
                ("end", Keyword),
                ("-- done", Comment),
            ]
        );
        // A built-in's name as a member is a member.
        assert_eq!(named("x.print")[2], ("print", Member));
        assert_eq!(named("x = math.pi")[2], ("math", Builtin));
    }

    #[test]
    fn the_spans_cover_the_source() {
        let source = "  a  =  1  ";
        let spans = spans(source);
        assert_eq!(spans.first().unwrap().0.start, 0);
        assert_eq!(spans.last().unwrap().0.end, source.len());
        assert!(spans.windows(2).all(|pair| pair[0].0.end == pair[1].0.start));
    }
}
