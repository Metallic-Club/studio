//! Finding and replacing in code (#554, #555): a [`Query`] (its text, and whether case matters, whole words only, or a
//! regular expression) found in a script's text, each match a byte range, and a match's replacement (`$1` and the like
//! for a regular expression's groups). The find bar of a script ([`FindBar`], [`super::find_bar`]) keeps its query
//! here.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use regex::{Regex, RegexBuilder};

use super::ScriptId;

/// What is looked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// The text, or the pattern when [`Query::regex`].
    pub text: String,
    /// Whether upper and lower case must match.
    pub case: bool,
    /// Whether a match must be a whole word: no letter, digit or `_` just before or after it.
    pub word: bool,
    /// Whether the text is a regular expression.
    pub regex: bool,
}

impl Query {
    /// The finder of this query: none for an empty one, an error saying why for a pattern that is not one.
    pub fn finder(&self) -> Result<Option<Finder>, String> {
        if self.text.is_empty() {
            return Ok(None);
        }
        let pattern = if self.regex {
            self.text.clone()
        } else {
            regex::escape(&self.text)
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|error| match error {
                regex::Error::Syntax(_) => "Not a regular expression".to_owned(),
                _ => "That pattern is too large".to_owned(),
            })?;
        Ok(Some(Finder {
            regex,
            word: self.word,
            regex_replace: self.regex,
        }))
    }
}

/// A query made ready to find.
#[derive(Clone, Debug)]
pub struct Finder {
    regex: Regex,
    word: bool,
    /// Whether a replacement's `$1` names a group: only for a regular expression.
    regex_replace: bool,
}

impl Finder {
    /// Every match in `text`, in order, as byte ranges. A match of nothing (`^`) is not one.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        self.regex
            .find_iter(text)
            .map(|found| found.range())
            .filter(|range| !range.is_empty())
            .filter(|range| !self.word || whole_word(text, range.clone()))
            .collect()
    }

    /// What the match `range` of `text` is replaced with for `with`: `with` itself, or for a regular expression `with`
    /// with its groups (`$1`, `${name}`) filled in.
    pub fn replacement(&self, text: &str, range: Range<usize>, with: &str) -> String {
        if !self.regex_replace {
            return with.to_owned();
        }
        match self.regex.captures_at(text, range.start) {
            Some(captures) if captures.get(0).is_some_and(|whole| whole.range() == range) => {
                let mut out = String::new();
                captures.expand(with, &mut out);
                out
            }
            _ => with.to_owned(),
        }
    }

    /// `text` with every one of `matches` (found by this finder, in order) replaced for `with`.
    pub fn replace_all(&self, text: &str, matches: &[Range<usize>], with: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for range in matches {
            out.push_str(&text[at..range.start]);
            out.push_str(&self.replacement(text, range.clone(), with));
            at = range.end;
        }
        out.push_str(&text[at..]);
        out
    }
}

/// Whether the bytes `range` of `text` are a whole word: no word character just before or after them.
fn whole_word(text: &str, range: Range<usize>) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !text[..range.start].chars().next_back().is_some_and(word) && !text[range.end..].chars().next().is_some_and(word)
}

/// The match of `matches` that is exactly `selection`, if one is.
pub fn current(matches: &[Range<usize>], selection: &Range<usize>) -> Option<usize> {
    matches.iter().position(|range| range == selection)
}

/// The match to go to next from `selection`: the one after it when it is a match, otherwise the first starting at or
/// after its start; past the last, the first.
pub fn next(matches: &[Range<usize>], selection: &Range<usize>) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if let Some(index) = current(matches, selection) {
        return Some((index + 1) % matches.len());
    }
    Some(
        matches
            .iter()
            .position(|range| range.start >= selection.start)
            .unwrap_or(0),
    )
}

/// The match to go to before `selection`: the one before it when it is a match, otherwise the last starting before its
/// start; before the first, the last.
pub fn previous(matches: &[Range<usize>], selection: &Range<usize>) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if let Some(index) = current(matches, selection) {
        return Some((index + matches.len() - 1) % matches.len());
    }
    Some(
        matches
            .iter()
            .rposition(|range| range.start < selection.start)
            .unwrap_or(matches.len() - 1),
    )
}

/// A script's find bar (#554): its query, shared by every script's bar as VS Code shares it, and which scripts have
/// it open.
#[derive(Debug, Default)]
pub struct FindBar {
    /// What is looked for.
    pub query: Query,
    /// What a match is replaced with.
    pub replace: String,
    /// Whether the replace row shows.
    pub show_replace: bool,
    /// The scripts whose bar is open.
    pub open: HashSet<ScriptId>,
    /// A script whose bar was just asked for: its query takes the keyboard, filled with the selection.
    pub asked: Option<ScriptId>,
    /// A script asked to go to its next match (`true`) or the one before (F3 and Ctrl+Shift+G).
    pub step: Option<(ScriptId, bool)>,
    /// Where each script's search is, as bytes: the match gone to, or the code's selection while it has the keyboard.
    /// Kept here because the code's own selection does not last while the bar has the keyboard.
    pub at: HashMap<ScriptId, Range<usize>>,
    /// The finder made from the query last, and the query it was made from.
    made: Option<(Query, Result<Option<Finder>, String>)>,
}

impl FindBar {
    /// Opens `script`'s bar (its replace row too, for `replace`), its query to take the keyboard.
    pub fn open(&mut self, script: ScriptId, replace: bool) {
        self.open.insert(script);
        self.show_replace |= replace;
        self.asked = Some(script);
    }

    /// Closes `script`'s bar.
    pub fn close(&mut self, script: ScriptId) {
        self.open.remove(&script);
    }

    /// Whether `script`'s bar is open.
    pub fn is_open(&self, script: ScriptId) -> bool {
        self.open.contains(&script)
    }

    /// The finder of the query, made again only when the query changed.
    pub fn finder(&mut self) -> Result<Option<Finder>, String> {
        if let Some((query, made)) = &self.made
            && *query == self.query
        {
            return made.clone();
        }
        let made = self.query.finder();
        self.made = Some((self.query.clone(), made.clone()));
        made
    }
}

/// The search across every script (#555), in the Search panel: its query, the replacement, and the files whose matches
/// are folded away.
#[derive(Debug, Default)]
pub struct Search {
    /// What is looked for.
    pub query: Query,
    /// What a match is replaced with.
    pub replace: String,
    /// Whether the replace field shows.
    pub show_replace: bool,
    /// Whether the query takes the keyboard next frame, all of it selected (the panel was just asked for).
    pub asked: bool,
    /// The files whose matches are folded away, by member.
    pub folded: HashSet<String>,
    /// The finder made from the query last, and the query it was made from.
    made: Option<(Query, Result<Option<Finder>, String>)>,
}

impl Search {
    /// The finder of the query, made again only when the query changed.
    pub fn finder(&mut self) -> Result<Option<Finder>, String> {
        if let Some((query, made)) = &self.made
            && *query == self.query
        {
            return made.clone();
        }
        let made = self.query.finder();
        self.made = Some((self.query.clone(), made.clone()));
        made
    }
}

/// The matches in one script file (#555).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    /// The file, a member of the project.
    pub member: String,
    /// The script it is the code of (the first, if scripts share it).
    pub script: ScriptId,
    /// That script's full name (`ServerScriptService.Door`).
    pub name: String,
    /// Its matches, in order.
    pub matches: Vec<LineMatch>,
}

/// A match and the line it is on, as a result shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// The match, as bytes of the file.
    pub range: Range<usize>,
    /// Its line, from 0.
    pub line: usize,
    /// Its column on the line, in characters from 0.
    pub column: usize,
    /// The line before it, without its indentation and cut short from the left when long.
    pub before: String,
    /// The match (up to the line's end, when it goes on).
    pub found: String,
    /// The rest of the line.
    pub after: String,
}

/// How much of a line before a match a result keeps.
const BEFORE: usize = 32;

/// The match `range` of `text` with its line, as a result shows it.
pub fn line_match(text: &str, range: Range<usize>) -> LineMatch {
    let start = text[..range.start].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[range.start..]
        .find('\n')
        .map_or(text.len(), |newline| range.start + newline);
    let line = text[..range.start].matches('\n').count();
    let before = text[start..range.start].trim_start();
    let kept = before.chars().count();
    let before = if kept > BEFORE {
        let skip = before.char_indices().nth(kept - BEFORE).map_or(0, |(byte, _)| byte);
        format!("…{}", &before[skip..])
    } else {
        before.to_owned()
    };
    let found_end = range.end.min(end);
    LineMatch {
        line,
        column: text[start..range.start].chars().count(),
        before,
        found: text[range.start..found_end].to_owned(),
        after: text[found_end..end].trim_end().to_owned(),
        range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    fn found(query: &Query, text: &str) -> Vec<String> {
        let finder = query.finder().unwrap().unwrap();
        finder
            .matches(text)
            .into_iter()
            .map(|range| text[range].to_owned())
            .collect()
    }

    #[test]
    fn text_is_found_as_written_unless_case_matters() {
        let text = "Part part PART (part) a.b";
        assert_eq!(found(&query("part"), text).len(), 4);
        let case = Query {
            case: true,
            ..query("part")
        };
        assert_eq!(found(&case, text), ["part", "part"]);
        // Not a pattern: the dot is a dot.
        assert_eq!(found(&query("a.b"), "a.b axb"), ["a.b"]);
        assert!(query("").finder().unwrap().is_none());
    }

    #[test]
    fn a_whole_word_has_no_word_character_beside_it() {
        let word = Query {
            word: true,
            ..query("part")
        };
        let text = "part parts _part part2 (part) part";
        let finder = word.finder().unwrap().unwrap();
        assert_eq!(finder.matches(text), [0..4, 24..28, 30..34]);
    }

    #[test]
    fn a_regular_expression_finds_and_replaces_with_its_groups() {
        let pattern = Query {
            regex: true,
            ..query(r"local (\w+) = (\d+)")
        };
        let text = "local a = 1\nlocal bb = 22\nlocal c = x";
        let finder = pattern.finder().unwrap().unwrap();
        let matches = finder.matches(text);
        assert_eq!(matches.len(), 2);
        assert_eq!(finder.replacement(text, matches[1].clone(), "$1 := $2"), "bb := 22");
        assert_eq!(finder.replace_all(text, &matches, "${2}=$1"), "1=a\n22=bb\nlocal c = x");
        // `^` matches nothing at each line start, which is not a match.
        let start = Query {
            regex: true,
            ..query("^")
        };
        assert!(start.finder().unwrap().unwrap().matches(text).is_empty());
        let bad = Query {
            regex: true,
            ..query("(")
        };
        assert_eq!(bad.finder().unwrap_err(), "Not a regular expression");
    }

    #[test]
    fn plain_text_replaces_as_written() {
        let finder = query("a").finder().unwrap().unwrap();
        let text = "a b a";
        let matches = finder.matches(text);
        assert_eq!(finder.replace_all(text, &matches, "$1"), "$1 b $1");
    }

    #[test]
    fn a_result_shows_its_line_around_the_match() {
        let text = "a = 1\n\t\tlocal door = open(door)\nb";
        let finder = query("door").finder().unwrap().unwrap();
        let matches = finder.matches(text);
        let first = line_match(text, matches[0].clone());
        assert_eq!(first.line, 1);
        assert_eq!(first.column, 8);
        assert_eq!((first.before.as_str(), first.found.as_str()), ("local ", "door"));
        assert_eq!(first.after, " = open(door)");
        // A long line is cut short before the match.
        let long = format!("{}x", "y".repeat(80));
        let far = line_match(&long, 80..81);
        assert!(far.before.starts_with('…') && far.before.chars().count() == BEFORE + 1);
        assert_eq!(far.found, "x");
    }

    #[test]
    fn next_and_previous_go_round() {
        let matches = [2..4, 10..12, 20..22];
        // From a match, the one after or before it.
        assert_eq!(next(&matches, &(10..12)), Some(2));
        assert_eq!(next(&matches, &(20..22)), Some(0));
        assert_eq!(previous(&matches, &(2..4)), Some(2));
        // From a cursor, the first at or after it, or the last before it.
        assert_eq!(next(&matches, &(5..5)), Some(1));
        assert_eq!(next(&matches, &(10..10)), Some(1));
        assert_eq!(next(&matches, &(30..30)), Some(0));
        assert_eq!(previous(&matches, &(15..15)), Some(1));
        assert_eq!(previous(&matches, &(0..0)), Some(2));
        assert_eq!(next(&[], &(0..0)), None);
        assert_eq!(current(&matches, &(10..12)), Some(1));
        assert_eq!(current(&matches, &(10..11)), None);
    }
}
