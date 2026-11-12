//! The script editor's settings (#529): its font, spacing, whitespace and colours, on Settings' Code Editor page, kept
//! in a file of their own, `code_editor.toml`, beside `settings.toml`.
//!
//! ```toml
//! [editor]
//! font = "jetbrains-mono"   # or "inter", or "file" with font_file
//! size = 13.0
//! line_spacing = 1.35
//! tab_width = 4
//! indent = "tabs"           # or "spaces"
//! show_whitespace = false
//! highlight_line = true
//! minimap = true
//!
//! [colors.dark]
//! keyword = "#F86D7C"
//! ```
//!
//! A key missing or not valid keeps its default, as `settings.toml`'s do. Each theme has its own colours: the page
//! edits those of the theme showing.

use toml::{Table, Value};

/// The file the script editor's settings are kept in, in Studio's config folder.
pub const CODE_EDITOR_FILE: &str = "code_editor.toml";

/// The code's size, in points.
pub const SIZE: std::ops::RangeInclusive<f32> = 8.0..=32.0;
/// The space between lines, as a multiple of the size.
pub const LINE_SPACING: std::ops::RangeInclusive<f32> = 1.0..=2.5;
/// The space added between letters, in points.
pub const LETTER_SPACING: std::ops::RangeInclusive<f32> = -1.0..=4.0;
/// The widths a tab may be, in spaces.
pub const TAB_WIDTHS: [u8; 3] = [2, 4, 8];

/// What a piece of code is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Names and anything else.
    Text,
    /// Reserved words: `local`, `function`, `end`...
    Keyword,
    /// `true`, `false` and `nil`.
    Constant,
    /// Luau's and Metallic's globals: `game`, `print`, `Vector3`, `math`...
    Builtin,
    /// A function's name, where it is called or made.
    Function,
    /// A field or method after `.` or `:`.
    Member,
    /// Strings.
    String,
    /// Numbers.
    Number,
    /// Comments.
    Comment,
    /// Operators and punctuation.
    Operator,
}

impl Kind {
    /// Every kind, in the order the page lists them.
    pub const ALL: [Kind; 10] = [
        Kind::Text,
        Kind::Keyword,
        Kind::Constant,
        Kind::Builtin,
        Kind::Function,
        Kind::Member,
        Kind::String,
        Kind::Number,
        Kind::Comment,
        Kind::Operator,
    ];

    /// Its name on the page.
    pub const fn label(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::Keyword => "Keywords",
            Kind::Constant => "true, false, nil",
            Kind::Builtin => "Built-in globals",
            Kind::Function => "Functions",
            Kind::Member => "Members",
            Kind::String => "Strings",
            Kind::Number => "Numbers",
            Kind::Comment => "Comments",
            Kind::Operator => "Operators",
        }
    }

    /// Its key in the file.
    fn key(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Keyword => "keyword",
            Kind::Constant => "constant",
            Kind::Builtin => "builtin",
            Kind::Function => "function",
            Kind::Member => "member",
            Kind::String => "string",
            Kind::Number => "number",
            Kind::Comment => "comment",
            Kind::Operator => "operator",
        }
    }

    fn index(self) -> usize {
        Kind::ALL.iter().position(|kind| *kind == self).unwrap_or(0)
    }
}

/// The colour of each [`Kind`], as sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette([[u8; 3]; Kind::ALL.len()]);

impl Palette {
    /// The dark theme's colours, after Roblox Studio's dark script editor.
    pub const DARK: Palette = Palette([
        [0xCC, 0xCC, 0xCC],
        [0xF8, 0x6D, 0x7C],
        [0xFF, 0xC6, 0x00],
        [0x84, 0xD6, 0xF7],
        [0xFD, 0xFB, 0xAC],
        [0x61, 0xA1, 0xF1],
        [0xAD, 0xF1, 0x95],
        [0xFF, 0xC6, 0x00],
        [0x66, 0x66, 0x66],
        [0xCC, 0xCC, 0xCC],
    ]);

    /// The light theme's colours, after Roblox Studio's light script editor.
    pub const LIGHT: Palette = Palette([
        [0x00, 0x00, 0x00],
        [0x00, 0x00, 0xFF],
        [0x00, 0x7F, 0x7F],
        [0x00, 0x00, 0x7F],
        [0x7F, 0x4F, 0x00],
        [0x00, 0x4F, 0x9F],
        [0x7F, 0x00, 0x7F],
        [0x00, 0x7F, 0x7F],
        [0x00, 0x7F, 0x00],
        [0x7F, 0x7F, 0x00],
    ]);

    /// The colour of `kind`.
    pub fn get(&self, kind: Kind) -> [u8; 3] {
        self.0[kind.index()]
    }

    /// Sets the colour of `kind`.
    pub fn set(&mut self, kind: Kind, rgb: [u8; 3]) {
        self.0[kind.index()] = rgb;
    }

    /// The colour of `kind`, for painting.
    pub fn color(&self, kind: Kind) -> egui::Color32 {
        let [r, g, b] = self.get(kind);
        egui::Color32::from_rgb(r, g, b)
    }
}

/// The code's typeface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CodeFont {
    /// JetBrains Mono, bundled.
    #[default]
    JetBrainsMono,
    /// Inter, bundled: proportional.
    Inter,
    /// A TrueType or OpenType file of the user's.
    File(String),
}

impl CodeFont {
    /// Its name on the page.
    pub fn label(&self) -> String {
        match self {
            CodeFont::JetBrainsMono => "JetBrains Mono".to_owned(),
            CodeFont::Inter => "Inter".to_owned(),
            CodeFont::File(path) => std::path::Path::new(path)
                .file_stem()
                .map_or_else(|| "A font file".to_owned(), |stem| stem.to_string_lossy().into_owned()),
        }
    }
}

/// The script editor's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeEditor {
    /// The typeface.
    pub font: CodeFont,
    /// The size, in points ([`SIZE`]).
    pub size: f32,
    /// The space between lines, as a multiple of the size ([`LINE_SPACING`]).
    pub line_spacing: f32,
    /// The space added between letters, in points ([`LETTER_SPACING`]).
    pub letter_spacing: f32,
    /// How many spaces wide a tab is ([`TAB_WIDTHS`]).
    pub tab_width: u8,
    /// Whether Tab puts in spaces rather than a tab.
    pub insert_spaces: bool,
    /// Whether spaces and tabs show, faintly.
    pub show_whitespace: bool,
    /// Whether the cursor's line is lit.
    pub highlight_line: bool,
    /// Whether the code's minimap shows down its right (#556).
    pub minimap: bool,
    /// The dark theme's colours.
    pub dark: Palette,
    /// The light theme's colours.
    pub light: Palette,
}

impl Default for CodeEditor {
    fn default() -> Self {
        Self {
            font: CodeFont::JetBrainsMono,
            size: 13.0,
            line_spacing: 1.35,
            letter_spacing: 0.0,
            tab_width: 4,
            insert_spaces: false,
            show_whitespace: false,
            highlight_line: true,
            minimap: true,
            dark: Palette::DARK,
            light: Palette::LIGHT,
        }
    }
}

impl CodeEditor {
    /// The colours of the dark theme, or the light one.
    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }

    /// The colours of the dark theme, or the light one, to change.
    pub fn palette_mut(&mut self, dark: bool) -> &mut Palette {
        if dark { &mut self.dark } else { &mut self.light }
    }

    /// What one press of Tab puts in.
    pub fn indent(&self) -> String {
        if self.insert_spaces {
            " ".repeat(usize::from(self.tab_width))
        } else {
            "\t".to_owned()
        }
    }

    /// Keeps every value in its range.
    pub fn clamp(&mut self) {
        let clamp = |value: f32, range: &std::ops::RangeInclusive<f32>| {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                *range.start()
            }
        };
        self.size = clamp(self.size, &SIZE);
        self.line_spacing = clamp(self.line_spacing, &LINE_SPACING);
        self.letter_spacing = clamp(self.letter_spacing, &LETTER_SPACING);
        if !TAB_WIDTHS.contains(&self.tab_width) {
            self.tab_width = 4;
        }
    }

    /// The settings `document` holds, the defaults for anything missing or not valid.
    pub fn from_document(document: &Table) -> Self {
        let mut settings = Self::default();
        if let Some(editor) = document.get("editor").and_then(Value::as_table) {
            let number = |key: &str| match editor.get(key) {
                Some(Value::Float(n)) => Some(*n as f32),
                Some(Value::Integer(n)) => Some(*n as f32),
                _ => None,
            };
            let flag = |key: &str| editor.get(key).and_then(Value::as_bool);
            match editor.get("font").and_then(Value::as_str) {
                Some("inter") => settings.font = CodeFont::Inter,
                Some("file") => {
                    if let Some(path) = editor.get("font_file").and_then(Value::as_str) {
                        settings.font = CodeFont::File(path.to_owned());
                    }
                }
                _ => {}
            }
            settings.size = number("size").unwrap_or(settings.size);
            settings.line_spacing = number("line_spacing").unwrap_or(settings.line_spacing);
            settings.letter_spacing = number("letter_spacing").unwrap_or(settings.letter_spacing);
            if let Some(width) = editor
                .get("tab_width")
                .and_then(Value::as_integer)
                .and_then(|width| u8::try_from(width).ok())
            {
                settings.tab_width = width;
            }
            settings.insert_spaces = editor.get("indent").and_then(Value::as_str) == Some("spaces");
            settings.show_whitespace = flag("show_whitespace").unwrap_or(settings.show_whitespace);
            settings.highlight_line = flag("highlight_line").unwrap_or(settings.highlight_line);
            settings.minimap = flag("minimap").unwrap_or(settings.minimap);
        }
        if let Some(colors) = document.get("colors").and_then(Value::as_table) {
            for (theme, palette) in [("dark", &mut settings.dark), ("light", &mut settings.light)] {
                let Some(table) = colors.get(theme).and_then(Value::as_table) else {
                    continue;
                };
                for kind in Kind::ALL {
                    if let Some(rgb) = table.get(kind.key()).and_then(Value::as_str).and_then(from_hex) {
                        palette.set(kind, rgb);
                    }
                }
            }
        }
        settings.clamp();
        settings
    }

    /// `base` (the file as read, whose other keys are kept) with these settings in it.
    pub fn to_document(&self, base: &Table) -> Table {
        let mut document = base.clone();
        with_section(&mut document, "editor", |editor| {
            let (font, file) = match &self.font {
                CodeFont::JetBrainsMono => ("jetbrains-mono", None),
                CodeFont::Inter => ("inter", None),
                CodeFont::File(path) => ("file", Some(path.clone())),
            };
            editor.insert("font".into(), Value::String(font.into()));
            match file {
                Some(path) => {
                    editor.insert("font_file".into(), Value::String(path));
                }
                None => {
                    editor.remove("font_file");
                }
            }
            editor.insert("size".into(), Value::Float(f64::from(self.size)));
            editor.insert("line_spacing".into(), Value::Float(f64::from(self.line_spacing)));
            editor.insert("letter_spacing".into(), Value::Float(f64::from(self.letter_spacing)));
            editor.insert("tab_width".into(), Value::Integer(i64::from(self.tab_width)));
            let indent = if self.insert_spaces { "spaces" } else { "tabs" };
            editor.insert("indent".into(), Value::String(indent.into()));
            editor.insert("show_whitespace".into(), Value::Boolean(self.show_whitespace));
            editor.insert("highlight_line".into(), Value::Boolean(self.highlight_line));
            editor.insert("minimap".into(), Value::Boolean(self.minimap));
        });
        with_section(&mut document, "colors", |colors| {
            for (theme, palette) in [("dark", &self.dark), ("light", &self.light)] {
                with_section(colors, theme, |table| {
                    for kind in Kind::ALL {
                        table.insert(kind.key().into(), Value::String(to_hex(palette.get(kind))));
                    }
                });
            }
        });
        document
    }
}

/// Changes the table `name` of `table` with `change`, making it if it is missing (or not a table).
fn with_section(table: &mut Table, name: &str, change: impl FnOnce(&mut Table)) {
    let mut inner = match table.remove(name) {
        Some(Value::Table(inner)) => inner,
        _ => Table::new(),
    };
    change(&mut inner);
    table.insert(name.to_owned(), Value::Table(inner));
}

/// `#RRGGBB`.
pub fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// The colour of `#RRGGBB` (the `#` optional, any case).
pub fn from_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_round_trip_and_keep_other_keys() {
        let mut settings = CodeEditor {
            font: CodeFont::File("C:/Fonts/Fira Code.ttf".into()),
            size: 15.0,
            line_spacing: 1.6,
            letter_spacing: 0.5,
            tab_width: 2,
            insert_spaces: true,
            show_whitespace: true,
            highlight_line: false,
            minimap: false,
            ..CodeEditor::default()
        };
        settings.dark.set(Kind::Keyword, [1, 2, 3]);
        settings.light.set(Kind::Comment, [0xAB, 0xCD, 0xEF]);
        let mut base = Table::new();
        base.insert("note".into(), Value::String("kept".into()));
        let document = settings.to_document(&base);
        assert_eq!(document.get("note").and_then(Value::as_str), Some("kept"));
        let text = document.to_string();
        let read = CodeEditor::from_document(&text.parse::<Table>().unwrap());
        assert_eq!(read, settings);
        assert_eq!(read.font.label(), "Fira Code");
        assert_eq!(read.indent(), "  ");
    }

    #[test]
    fn values_out_of_range_or_not_valid_are_not_used() {
        let document: Table = "[editor]\nsize = 400\nline_spacing = \"wide\"\ntab_width = 3\nfont = \"comic\"\n\
                               [colors.dark]\nkeyword = \"red\"\nstring = \"#00ff00\""
            .parse()
            .unwrap();
        let read = CodeEditor::from_document(&document);
        let defaults = CodeEditor::default();
        assert_eq!(read.size, *SIZE.end());
        assert_eq!(read.line_spacing, defaults.line_spacing);
        assert_eq!(read.tab_width, 4);
        assert_eq!(read.font, CodeFont::JetBrainsMono);
        assert_eq!(read.dark.get(Kind::Keyword), Palette::DARK.get(Kind::Keyword));
        assert_eq!(read.dark.get(Kind::String), [0, 0xFF, 0]);
        assert_eq!(CodeEditor::default().indent(), "\t");
    }
}
