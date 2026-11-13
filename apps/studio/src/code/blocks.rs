//! Enter after a line that opens a block (#542): the next line indented one step more and, when the block is not
//! closed yet, its `end` (or `until`) under it, as Roblox Studio's script editor does.
//!
//! A line opens a block when it ends in `then` (of an `if`, not an `elseif`), `do` (of `for`, `while` or a bare
//! `do`), `repeat`, or the `)` that closes a `function`'s parameters, named or not. An anonymous function inside a
//! call's brackets, `x.Touched:Connect(function(hit)`, is closed with `end)`. Whether the block is closed is a count
//! over the whole script: more `function`, `if` and `do` than `end` (or more `repeat` than `until`) means one is open.
//!
//! A block comment is closed the same way (#575): Enter after a line that opens `--[[` (or `--[==[`) not closed
//! anywhere after it steps in and puts the `]]` (or `]==]`) under it.

use super::lexer::{Token, TokenKind, lex};

/// What Enter puts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enter {
    /// The bytes of the text it replaces: from the cursor to the end of the line, when what follows the cursor is
    /// carried under the `end` (brackets that close the line), or just the cursor.
    pub replace: std::ops::Range<usize>,
    /// What goes in their place.
    pub with: String,
    /// Where the cursor goes, in bytes into [`Self::with`].
    pub cursor: usize,
}

/// How a block is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Closer {
    End,
    Until,
}

/// What Enter at byte `at` of `text` puts in when the line there opens a block, indenting by `unit`; `None` when it
/// does not (Enter keeps the line's indentation, as always).
pub fn enter(text: &str, at: usize, unit: &str) -> Option<Enter> {
    let at = at.min(text.len());
    let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |offset| at + offset);
    let before = &text[line_start..at];
    let after = &text[at..line_end];
    // Only at the end of the line, or before the brackets that close it.
    if !after.chars().all(|c| c == ')' || c.is_whitespace()) {
        return None;
    }
    let indent: String = before.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    if let Some(level) = open_comment(text, line_start, at) {
        let inner = format!("\n{indent}{unit}");
        return Some(Enter {
            replace: at..at,
            with: format!("{inner}\n{indent}]{}]", "=".repeat(level)),
            cursor: inner.len(),
        });
    }
    let tokens: Vec<Token> = lex(before)
        .into_iter()
        .filter(|token| token.kind != TokenKind::Comment)
        .collect();
    let (closer, brackets) = opens(before, &tokens)?;
    let inner = format!("\n{indent}{unit}");
    let cursor = inner.len();
    if !unclosed(text, closer) {
        // Closed already: only the step in.
        return Some(Enter {
            replace: at..at,
            with: inner,
            cursor,
        });
    }
    let word = match closer {
        Closer::End => "end",
        Closer::Until => "until ",
    };
    // What follows the cursor (the call's `)`) goes after the `end`; with nothing there, the brackets still open.
    let tail = after.trim();
    let tail = if tail.is_empty() {
        ")".repeat(brackets)
    } else {
        tail.to_owned()
    };
    Some(Enter {
        replace: at..line_end,
        with: format!("{inner}\n{indent}{word}{tail}"),
        cursor,
    })
}

/// The level of the block comment (`--[[` is 0, `--[==[` is 2) that opens on the line from byte `line_start` of `text`
/// before byte `at` and is not closed (#575); `None` when there is none.
fn open_comment(text: &str, line_start: usize, at: usize) -> Option<usize> {
    lex(text)
        .into_iter()
        .filter(|token| token.kind == TokenKind::Comment && token.unterminated)
        .filter(|token| (line_start..at).contains(&token.start))
        .find_map(|token| {
            let opener = text.get(token.start..)?.strip_prefix("--[")?;
            let level = opener.chars().take_while(|c| *c == '=').count();
            let bracket = token.start + 3 + level;
            // A long comment's bracket, and the cursor past it.
            (opener[level..].starts_with('[') && at > bracket).then_some(level)
        })
}

/// How the block `tokens` (of the line `line`, up to the cursor) opens is closed, and how many brackets were open
/// before its `function`; `None` when the line opens no block.
fn opens(line: &str, tokens: &[Token]) -> Option<(Closer, usize)> {
    let last = tokens.last()?;
    let first = |word: &str| tokens.iter().any(|token| token.is(line, word));
    if last.is(line, "then") {
        // `elseif ... then` continues the `if`'s block.
        return (first("if") && !tokens.first().is_some_and(|token| token.is(line, "elseif")))
            .then_some((Closer::End, 0));
    }
    if last.is(line, "do") {
        return Some((Closer::End, 0));
    }
    if last.is(line, "repeat") {
        return Some((Closer::Until, 0));
    }
    if last.is(line, ")") {
        let function = tokens.iter().rposition(|token| token.is(line, "function"))?;
        // `function name(...)`, `function a.b:c(...)` or `function(...)`: the `)` closes the first `(` after it.
        let open = function + tokens[function..].iter().position(|token| token.is(line, "("))?;
        if !tokens[function + 1..open]
            .iter()
            .all(|token| token.kind == TokenKind::Name || token.is(line, ".") || token.is(line, ":"))
        {
            return None;
        }
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(open) {
            if token.is(line, "(") {
                depth += 1;
            } else if token.is(line, ")") {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    if index != tokens.len() - 1 {
                        return None;
                    }
                    break;
                }
            }
        }
        // The brackets the function is inside, on this line.
        let brackets = tokens[..function].iter().fold(0usize, |open, token| {
            if token.is(line, "(") {
                open + 1
            } else if token.is(line, ")") {
                open.saturating_sub(1)
            } else {
                open
            }
        });
        return Some((Closer::End, brackets));
    }
    None
}

/// Whether `text` has a block closed by `closer` that is not closed.
fn unclosed(text: &str, closer: Closer) -> bool {
    let tokens = lex(text);
    let count = |word: &str| tokens.iter().filter(|token| token.is(text, word)).count();
    match closer {
        Closer::End => count("function") + count("if") + count("do") > count("end"),
        Closer::Until => count("repeat") > count("until"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enter at the end of `text` (or at `|`), then the text and where the cursor is (`|`).
    fn typed(text: &str) -> String {
        let at = text.find('|').unwrap_or(text.len());
        let text = text.replace('|', "");
        let mut out = text.clone();
        match enter(&text, at, "\t") {
            Some(enter) => {
                out.replace_range(enter.replace.clone(), &enter.with);
                out.insert(enter.replace.start + enter.cursor, '|');
            }
            None => out.insert_str(at, "\n|"),
        }
        out
    }

    #[test]
    fn a_block_is_closed_under_its_opening_line() {
        assert_eq!(typed("function open(door)"), "function open(door)\n\t|\nend");
        assert_eq!(typed("local function f()"), "local function f()\n\t|\nend");
        assert_eq!(typed("function Doors.open(by)"), "function Doors.open(by)\n\t|\nend");
        assert_eq!(typed("function Door:Open()"), "function Door:Open()\n\t|\nend");
        assert_eq!(typed("if ready then"), "if ready then\n\t|\nend");
        assert_eq!(typed("for i = 1, 10 do"), "for i = 1, 10 do\n\t|\nend");
        assert_eq!(typed("while true do"), "while true do\n\t|\nend");
        assert_eq!(typed("do"), "do\n\t|\nend");
        assert_eq!(typed("repeat"), "repeat\n\t|\nuntil ");
        assert_eq!(typed("local f = function(a, b)"), "local f = function(a, b)\n\t|\nend");
        // Indented, and with a comment after it.
        assert_eq!(
            typed("x = 1\n  if a then -- check"),
            "x = 1\n  if a then -- check\n  \t|\n  end"
        );
    }

    #[test]
    fn an_anonymous_function_in_a_call_is_closed_with_its_brackets() {
        assert_eq!(
            typed("part.Touched:Connect(function(hit)"),
            "part.Touched:Connect(function(hit)\n\t|\nend)"
        );
        // The `)` after the cursor goes under, after the `end`.
        assert_eq!(typed("task.spawn(function()|)"), "task.spawn(function()\n\t|\nend)");
    }

    #[test]
    fn a_closed_block_only_steps_in() {
        assert_eq!(typed("function f()|\nend"), "function f()\n\t|\nend");
        assert_eq!(typed("if a then|\n\tb()\nend"), "if a then\n\t|\n\tb()\nend");
        assert_eq!(typed("repeat|\nuntil done"), "repeat\n\t|\nuntil done");
    }

    /// #575: a block comment not closed is closed under its opening line, at its level.
    #[test]
    fn a_block_comment_is_closed_under_its_opening_line() {
        assert_eq!(typed("--[["), "--[[\n\t|\n]]");
        assert_eq!(typed("x = 1\n  --[[ notes"), "x = 1\n  --[[ notes\n  \t|\n  ]]");
        assert_eq!(typed("--[==["), "--[==[\n\t|\n]==]");
        assert_eq!(typed("print(1) --[["), "print(1) --[[\n\t|\n]]");
        // Closed already, on the line or below: a plain Enter.
        assert_eq!(typed("--[[ a ]]"), "--[[ a ]]\n|");
        assert_eq!(typed("--[[|\nold\n]]"), "--[[\n|\nold\n]]");
        // Not a block comment, or the cursor before its bracket.
        assert_eq!(typed("-- [["), "-- [[\n|");
        assert_eq!(typed("local s = \"--[[\""), "local s = \"--[[\"\n|");
    }

    #[test]
    fn other_lines_are_left_to_the_plain_enter() {
        assert_eq!(typed("print(1)"), "print(1)\n|");
        assert_eq!(typed("if a then\nelseif b then"), "if a then\nelseif b then\n|");
        assert_eq!(typed("f(function() end)"), "f(function() end)\n|");
        assert_eq!(typed("if a| then"), "if a\n| then");
        assert_eq!(typed("-- function f()"), "-- function f()\n|");
        assert_eq!(typed("local s = \"if a then\""), "local s = \"if a then\"\n|");
        assert_eq!(typed("call(a)"), "call(a)\n|");
    }
}
