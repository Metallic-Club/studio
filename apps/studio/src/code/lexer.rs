//! Luau's tokens (#529): what the script editor colours, and what its analysis reads (#530-#532).
//!
//! The lexer never fails. Code being typed is often unfinished: an unterminated string or comment runs to the end of
//! its line (or the file, for a long one) and is flagged ([`Token::unterminated`]); a character Luau does not know is an
//! [`TokenKind::Unknown`] token of its own.

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// A reserved word, such as `local` or `end`, or `true`, `false` and `nil` (see [`Token::is_constant`]).
    Keyword,
    /// A name: a variable, a field, a function.
    Name,
    /// A number, such as `12`, `0xFF` or `1e-3`.
    Number,
    /// A string, quoted, long (`[[...]]`) or interpolated (`` `...` ``).
    String,
    /// A comment, to the end of the line or long (`--[[...]]`).
    Comment,
    /// An operator or punctuation: `+`, `..`, `==`, `(`, `,`, `.`, ...
    Symbol,
    /// A character Luau does not use.
    Unknown,
}

/// One token: its kind and where it is, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    /// What it is.
    pub kind: TokenKind,
    /// Its first byte.
    pub start: usize,
    /// One past its last byte.
    pub end: usize,
    /// A string or comment the file ends (or, a quoted string, its line ends) before closing.
    pub unterminated: bool,
}

impl Token {
    /// Its text in `source`.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.start..self.end).unwrap_or("")
    }

    /// Whether it is `true`, `false` or `nil`.
    pub fn is_constant(&self, source: &str) -> bool {
        self.kind == TokenKind::Keyword && matches!(self.text(source), "true" | "false" | "nil")
    }

    /// Whether it is the symbol `symbol`.
    pub fn is(&self, source: &str, symbol: &str) -> bool {
        matches!(self.kind, TokenKind::Symbol | TokenKind::Keyword) && self.text(source) == symbol
    }
}

/// Luau's reserved words.
pub const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or",
    "repeat", "return", "then", "true", "until", "while", "continue",
];

/// Luau's operators and punctuation of more than one character, longest first.
const SYMBOLS: [&str; 17] = [
    "...", "//=", "..=", "==", "~=", "<=", ">=", "..", "::", "->", "+=", "-=", "*=", "/=", "%=", "^=", "//",
];

/// The tokens of `source`, comments included, in order.
pub fn lex(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if c.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        let (kind, end, unterminated) = if c == b'-' && bytes.get(at + 1) == Some(&b'-') {
            // A comment: long if `[[` or `[==[` follows.
            match long_bracket(bytes, at + 2) {
                Some(level) => {
                    let (end, closed) = close_long(bytes, at + 2, level);
                    (TokenKind::Comment, end, !closed)
                }
                None => (TokenKind::Comment, line_end(bytes, at), false),
            }
        } else if c == b'['
            && let Some(level) = long_bracket(bytes, at)
        {
            let (end, closed) = close_long(bytes, at, level);
            (TokenKind::String, end, !closed)
        } else if c == b'"' || c == b'\'' {
            let (end, closed) = close_quoted(bytes, at, c);
            (TokenKind::String, end, !closed)
        } else if c == b'`' {
            let (end, closed) = close_interpolated(bytes, at);
            (TokenKind::String, end, !closed)
        } else if c.is_ascii_digit() || (c == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)) {
            (TokenKind::Number, number_end(bytes, at), false)
        } else if c == b'_' || c.is_ascii_alphabetic() {
            let mut end = at + 1;
            while end < bytes.len() && (bytes[end] == b'_' || bytes[end].is_ascii_alphanumeric()) {
                end += 1;
            }
            let word = &source[at..end];
            let kind = if KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else {
                TokenKind::Name
            };
            (kind, end, false)
        } else if let Some(symbol) = SYMBOLS.iter().find(|symbol| bytes[at..].starts_with(symbol.as_bytes())) {
            (TokenKind::Symbol, at + symbol.len(), false)
        } else if b"+-*/%^#&|~<>=(){}[];:,.?!@".contains(&c) {
            (TokenKind::Symbol, at + 1, false)
        } else {
            // One character, whole (not one byte of it).
            let len = source[at..].chars().next().map_or(1, char::len_utf8);
            (TokenKind::Unknown, at + len, false)
        };
        tokens.push(Token {
            kind,
            start,
            end,
            unterminated,
        });
        at = end;
    }
    tokens
}

/// Where the line holding byte `at` ends (its `\n`, or the end).
fn line_end(bytes: &[u8], at: usize) -> usize {
    bytes[at..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |offset| at + offset)
}

/// The level of a long bracket (`[[` is 0, `[==[` is 2) opening at `at`, if one does.
fn long_bracket(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut level = 0;
    while bytes.get(at + 1 + level) == Some(&b'=') {
        level += 1;
    }
    (bytes.get(at + 1 + level) == Some(&b'[')).then_some(level)
}

/// The end of the long string or comment whose bracket of `level` opens at `at`, and whether it closes.
fn close_long(bytes: &[u8], at: usize, level: usize) -> (usize, bool) {
    let mut close = Vec::with_capacity(level + 2);
    close.push(b']');
    close.extend(std::iter::repeat_n(b'=', level));
    close.push(b']');
    let body = at + level + 2;
    match bytes[body.min(bytes.len())..]
        .windows(close.len())
        .position(|window| window == close.as_slice())
    {
        Some(offset) => (body + offset + close.len(), true),
        None => (bytes.len(), false),
    }
}

/// The end of the string quoted with `quote` opening at `at`, and whether it closes on its line.
fn close_quoted(bytes: &[u8], at: usize, quote: u8) -> (usize, bool) {
    let mut end = at + 1;
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'\n' => return (end, false),
            b if b == quote => return (end + 1, true),
            _ => end += 1,
        }
    }
    (bytes.len(), false)
}

/// The end of the interpolated string opening at `at` (its `{...}` parts are in it), and whether it closes.
fn close_interpolated(bytes: &[u8], at: usize) -> (usize, bool) {
    let mut end = at + 1;
    let mut depth = 0usize;
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'{' => {
                depth += 1;
                end += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                end += 1;
            }
            b'\n' if depth == 0 => return (end, false),
            b'`' if depth == 0 => return (end + 1, true),
            _ => end += 1,
        }
    }
    (bytes.len(), false)
}

/// The end of the number starting at `at`.
fn number_end(bytes: &[u8], at: usize) -> usize {
    let mut end = at;
    let hex = bytes[at] == b'0' && matches!(bytes.get(at + 1), Some(b'x' | b'X' | b'b' | b'B'));
    if hex {
        end += 2;
        while end < bytes.len() && (bytes[end].is_ascii_hexdigit() || bytes[end] == b'_') {
            end += 1;
        }
        return end;
    }
    while end < bytes.len() {
        let b = bytes[end];
        let exponent_sign = matches!(b, b'+' | b'-') && end > at && matches!(bytes[end - 1], b'e' | b'E');
        if b.is_ascii_digit() || b == b'_' || b == b'.' || matches!(b, b'e' | b'E') || exponent_sign {
            // `1..2` is a number, a concatenation, and a number.
            if b == b'.' && bytes.get(end + 1) == Some(&b'.') {
                break;
            }
            end += 1;
        } else {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(TokenKind, &str)> {
        lex(source)
            .iter()
            .map(|token| (token.kind, token.text(source)))
            .collect()
    }

    #[test]
    fn words_numbers_strings_and_symbols() {
        use TokenKind::*;
        assert_eq!(
            kinds("local x = 0x1F + 1.5e-3 .. \"a\\\"b\" -- note\nx += #t"),
            [
                (Keyword, "local"),
                (Name, "x"),
                (Symbol, "="),
                (Number, "0x1F"),
                (Symbol, "+"),
                (Number, "1.5e-3"),
                (Symbol, ".."),
                (String, "\"a\\\"b\""),
                (Comment, "-- note"),
                (Name, "x"),
                (Symbol, "+="),
                (Symbol, "#"),
                (Name, "t"),
            ]
        );
        assert_eq!(kinds("1..2"), [(Number, "1"), (Symbol, ".."), (Number, "2")]);
        assert_eq!(kinds("a:b(...)")[4], (Symbol, "..."));
    }

    #[test]
    fn long_strings_and_comments_and_interpolation() {
        use TokenKind::*;
        let source = "--[==[ a ]] b ]==] x = [[\nline]] y = `hi {name}!`";
        assert_eq!(
            kinds(source),
            [
                (Comment, "--[==[ a ]] b ]==]"),
                (Name, "x"),
                (Symbol, "="),
                (String, "[[\nline]]"),
                (Name, "y"),
                (Symbol, "="),
                (String, "`hi {name}!`"),
            ]
        );
    }

    #[test]
    fn unfinished_code_still_lexes() {
        let source = "print(\"open\nnext --[[ never closed";
        let tokens = lex(source);
        let string = tokens[2];
        assert_eq!(
            (string.kind, string.text(source), string.unterminated),
            (TokenKind::String, "\"open", true)
        );
        let comment = *tokens.last().unwrap();
        assert!(comment.kind == TokenKind::Comment && comment.unterminated);
        assert_eq!(comment.end, source.len());
        // Not Luau, but kept whole.
        assert_eq!(kinds("é")[0], (TokenKind::Unknown, "é"));
        assert!(lex("").is_empty());
    }
}
