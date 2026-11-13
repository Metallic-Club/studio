//! Several cursors at once (#558): Alt+click adds a cursor, and typing, deleting, Enter, Tab, pasting, copying and the
//! arrows act at every one, as VS Code does. Escape, or a click without Alt, goes back to one.
//!
//! The code field keeps its own cursor, the first; the others ([`Caret`]s) are kept beside it, and while there are any
//! the field's keys are done here, at every cursor ([`edit`], [`motion`]). Shift+Alt makes a cursor on each line of a
//! box (#559, [`column`]).

use std::ops::Range;

/// A cursor: where its selection started (`anchor`) and where it is (`head`), as character indices. Without a
/// selection the two are the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Caret {
    /// Where the selection started; it stays when Shift moves the cursor.
    pub anchor: usize,
    /// Where the cursor is.
    pub head: usize,
}

impl Caret {
    /// A cursor at `at` with nothing selected.
    pub fn at(at: usize) -> Self {
        Self { anchor: at, head: at }
    }

    /// What it selects, first to last.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// Whether it selects nothing.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// An edit made at every cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Puts text in place of each selection.
    Insert(String),
    /// Puts each of these in place of the selection of the cursor at the same place in document order: a paste with
    /// a line per cursor.
    InsertEach(Vec<String>),
    /// Deletes each selection, or the character (`word`: the word) before each cursor.
    Backspace {
        /// A word at a time (Ctrl).
        word: bool,
    },
    /// Deletes each selection, or the character (`word`: the word) after each cursor.
    Delete {
        /// A word at a time (Ctrl).
        word: bool,
    },
    /// A new line at each cursor, with its line's indentation.
    Newline,
}

/// A way of moving every cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// A character (`word`: a word) left.
    Left {
        /// A word at a time (Ctrl).
        word: bool,
    },
    /// A character (`word`: a word) right.
    Right {
        /// A word at a time (Ctrl).
        word: bool,
    },
    /// A line up.
    Up,
    /// A line down.
    Down,
    /// To the line's first non-blank character, or its start when already there.
    Home,
    /// To the line's end.
    End,
}

/// Whether `c` is part of a word.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the line holding character `at` of `chars`.
fn line_start(chars: &[char], at: usize) -> usize {
    chars[..at.min(chars.len())]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |newline| newline + 1)
}

/// The end of the line holding character `at` of `chars` (before its newline).
fn line_end(chars: &[char], at: usize) -> usize {
    let at = at.min(chars.len());
    chars[at..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |newline| at + newline)
}

/// The start of the word (or run of other characters) before `at`, past the blanks before it.
fn word_left(chars: &[char], at: usize) -> usize {
    let mut at = at.min(chars.len());
    while at > 0 && chars[at - 1].is_whitespace() && chars[at - 1] != '\n' {
        at -= 1;
    }
    if at > 0 && chars[at - 1] == '\n' {
        return at - 1;
    }
    let kind = at.checked_sub(1).map(|before| is_word(chars[before]));
    while at > 0 && Some(is_word(chars[at - 1])) == kind && !chars[at - 1].is_whitespace() {
        at -= 1;
    }
    at
}

/// The end of the word (or run of other characters) after `at`, past the blanks before it.
fn word_right(chars: &[char], at: usize) -> usize {
    let mut at = at.min(chars.len());
    while at < chars.len() && chars[at].is_whitespace() && chars[at] != '\n' {
        at += 1;
    }
    if at < chars.len() && chars[at] == '\n' {
        return at + 1;
    }
    let kind = chars.get(at).map(|c| is_word(*c));
    while at < chars.len() && Some(is_word(chars[at])) == kind && !chars[at].is_whitespace() {
        at += 1;
    }
    at
}

/// Keeps every cursor within a text `length` characters long: the text can change under cursors kept from before
/// (a replace from the Search panel).
fn clamp(carets: &mut [Caret], length: usize) {
    for caret in carets {
        caret.anchor = caret.anchor.min(length);
        caret.head = caret.head.min(length);
    }
}

/// Puts the cursors that overlap or meet together, keeping the order of the first of each.
pub fn merge(carets: &mut Vec<Caret>) {
    let mut out: Vec<Caret> = Vec::with_capacity(carets.len());
    for caret in carets.drain(..) {
        let range = caret.range();
        match out.iter_mut().find(|other| {
            let theirs = other.range();
            (range.start < theirs.end && theirs.start < range.end) || other.head == caret.head
        }) {
            Some(other) => {
                let theirs = other.range();
                let (start, end) = (theirs.start.min(range.start), theirs.end.max(range.end));
                *other = if other.head >= other.anchor {
                    Caret {
                        anchor: start,
                        head: end,
                    }
                } else {
                    Caret {
                        anchor: end,
                        head: start,
                    }
                };
            }
            None => out.push(caret),
        }
    }
    *carets = out;
}

/// Makes `edit` at every one of `carets` in `text`, each ending after what it put in. `indent` is what Tab puts in.
pub fn edit(text: &mut String, carets: &mut Vec<Caret>, edit: &Edit) {
    let chars: Vec<char> = text.chars().collect();
    clamp(carets, chars.len());
    // Each cursor's range to replace and what replaces it, by its place in `carets`.
    let mut order: Vec<usize> = (0..carets.len()).collect();
    order.sort_by_key(|&index| carets[index].range().start);
    let mut changes: Vec<(usize, Range<usize>, String)> = Vec::new();
    for (place, &index) in order.iter().enumerate() {
        let caret = carets[index];
        let range = caret.range();
        let (range, with) = match edit {
            Edit::Insert(with) => (range, with.clone()),
            Edit::InsertEach(each) => (range, each.get(place).cloned().unwrap_or_default()),
            Edit::Backspace { .. } | Edit::Delete { .. } if !caret.is_empty() => (range, String::new()),
            Edit::Backspace { word } => {
                let from = if *word {
                    word_left(&chars, caret.head)
                } else {
                    caret.head.saturating_sub(1)
                };
                (from..caret.head, String::new())
            }
            Edit::Delete { word } => {
                let to = if *word {
                    word_right(&chars, caret.head)
                } else {
                    (caret.head + 1).min(chars.len())
                };
                (caret.head..to, String::new())
            }
            Edit::Newline => {
                let start = line_start(&chars, range.start);
                let indent: String = chars[start..]
                    .iter()
                    .take_while(|c| **c == ' ' || **c == '\t')
                    .collect();
                (range, format!("\n{indent}"))
            }
        };
        changes.push((index, range, with));
    }
    // Replaced first to last; a range overlapping the one before it starts where that ends.
    changes.sort_by_key(|(_, range, _)| range.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut shift: isize = 0;
    for (index, range, with) in &changes {
        let start = range.start.max(at);
        let end = range.end.max(start);
        out.extend(&chars[at..start]);
        out.push_str(with);
        let added = with.chars().count();
        let new_start = (start as isize + shift) as usize;
        carets[*index] = Caret::at(new_start + added);
        shift += added as isize - (end - start) as isize;
        at = end;
    }
    out.extend(&chars[at.min(chars.len())..]);
    *text = out;
    merge(carets);
}

/// Moves every one of `carets` in `text` by `motion`, their selections growing with `extend` (Shift).
pub fn motion(text: &str, carets: &mut Vec<Caret>, motion: Motion, extend: bool) {
    let chars: Vec<char> = text.chars().collect();
    clamp(carets, chars.len());
    for caret in carets.iter_mut() {
        let range = caret.range();
        let head = match motion {
            // Without Shift, a selection collapses to its side.
            Motion::Left { .. } if !extend && !caret.is_empty() => range.start,
            Motion::Right { .. } if !extend && !caret.is_empty() => range.end,
            Motion::Left { word: true } => word_left(&chars, caret.head),
            Motion::Left { word: false } => caret.head.saturating_sub(1),
            Motion::Right { word: true } => word_right(&chars, caret.head),
            Motion::Right { word: false } => (caret.head + 1).min(chars.len()),
            Motion::Up | Motion::Down => {
                let start = line_start(&chars, caret.head);
                let column = caret.head - start;
                if motion == Motion::Up {
                    if start == 0 {
                        0
                    } else {
                        let above = line_start(&chars, start - 1);
                        (above + column).min(start - 1)
                    }
                } else {
                    let end = line_end(&chars, caret.head);
                    if end >= chars.len() {
                        chars.len()
                    } else {
                        let below = end + 1;
                        (below + column).min(line_end(&chars, below))
                    }
                }
            }
            Motion::Home => {
                let start = line_start(&chars, caret.head);
                let text_start = start + chars[start..].iter().take_while(|c| **c == ' ' || **c == '\t').count();
                if caret.head == text_start { start } else { text_start }
            }
            Motion::End => line_end(&chars, caret.head),
        };
        caret.head = head;
        if !extend {
            caret.anchor = head;
        }
    }
    merge(carets);
}

/// What the cursors select in `text`, in document order, each its own string.
pub fn selections(text: &str, carets: &[Caret]) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut ranges: Vec<Range<usize>> = carets.iter().map(Caret::range).collect();
    ranges.sort_by_key(|range| range.start);
    ranges
        .into_iter()
        .map(|range| {
            chars[range.start.min(chars.len())..range.end.min(chars.len())]
                .iter()
                .collect()
        })
        .collect()
}

/// What a paste of `pasted` puts at `carets`: a line at each cursor when it has as many lines as there are cursors,
/// as VS Code does, otherwise all of it at each.
pub fn paste(pasted: &str, carets: usize) -> Edit {
    let lines: Vec<&str> = pasted.lines().collect();
    if carets > 1 && lines.len() == carets {
        Edit::InsertEach(lines.into_iter().map(str::to_owned).collect())
    } else {
        Edit::Insert(pasted.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carets(at: &[usize]) -> Vec<Caret> {
        at.iter().map(|&at| Caret::at(at)).collect()
    }

    fn heads(carets: &[Caret]) -> Vec<usize> {
        carets.iter().map(|caret| caret.head).collect()
    }

    #[test]
    fn typing_goes_in_at_every_cursor() {
        let mut text = "ab\ncd\nef".to_owned();
        let mut at = carets(&[3, 0, 6]);
        edit(&mut text, &mut at, &Edit::Insert("x".into()));
        assert_eq!(text, "xab\nxcd\nxef");
        // Each keeps its place in the list.
        assert_eq!(heads(&at), [5, 1, 9]);
        edit(&mut text, &mut at, &Edit::Backspace { word: false });
        assert_eq!(text, "ab\ncd\nef");
        assert_eq!(heads(&at), [3, 0, 6]);
    }

    #[test]
    fn a_selection_is_replaced_and_deleted() {
        let mut text = "local a = 1\nlocal b = 2".to_owned();
        let mut at = vec![Caret { anchor: 0, head: 5 }, Caret { anchor: 12, head: 17 }];
        edit(&mut text, &mut at, &Edit::Insert("var".into()));
        assert_eq!(text, "var a = 1\nvar b = 2");
        let mut at = vec![Caret { anchor: 4, head: 5 }, Caret { anchor: 14, head: 15 }];
        edit(&mut text, &mut at, &Edit::Delete { word: false });
        assert_eq!(text, "var  = 1\nvar  = 2");
        edit(&mut text, &mut at, &Edit::Delete { word: false });
        assert_eq!(text, "var = 1\nvar = 2");
    }

    #[test]
    fn enter_keeps_each_line_s_indentation_and_words_go_whole() {
        let mut text = "\tif a then\n  b()".to_owned();
        let mut at = carets(&[10, 16]);
        edit(&mut text, &mut at, &Edit::Newline);
        assert_eq!(text, "\tif a then\n\t\n  b()\n  ");
        let mut text = "local door = open".to_owned();
        let mut at = carets(&[10, 17]);
        edit(&mut text, &mut at, &Edit::Backspace { word: true });
        assert_eq!(text, "local  = ");
    }

    #[test]
    fn cursors_past_the_end_stay_in_the_text() {
        let mut text = "ab".to_owned();
        let mut at = vec![Caret::at(9), Caret { anchor: 1, head: 30 }];
        edit(&mut text, &mut at, &Edit::Backspace { word: false });
        assert_eq!(text, "a");
        motion(&text, &mut at, Motion::Up, false);
        assert!(at.iter().all(|caret| caret.head <= 1));
    }

    #[test]
    fn cursors_that_meet_become_one() {
        let mut text = "abc".to_owned();
        let mut at = carets(&[1, 2]);
        edit(&mut text, &mut at, &Edit::Backspace { word: false });
        assert_eq!(text, "c");
        assert_eq!(at, [Caret::at(0)]);
    }

    #[test]
    fn the_arrows_move_every_cursor() {
        let text = "abc\nde\n\tfghi";
        let mut at = carets(&[2, 6]);
        motion(text, &mut at, Motion::Down, false);
        assert_eq!(heads(&at), [6, 9]);
        motion(text, &mut at, Motion::Up, false);
        assert_eq!(heads(&at), [2, 6]);
        motion(text, &mut at, Motion::End, true);
        assert_eq!(at, [Caret { anchor: 2, head: 3 }, Caret::at(6)]);
        // Without Shift a selection collapses to its side.
        motion(text, &mut at, Motion::Left { word: false }, false);
        assert_eq!(heads(&at), [2, 5]);
        let mut at = carets(&[11]);
        motion(text, &mut at, Motion::Home, false);
        assert_eq!(heads(&at), [8]);
        motion(text, &mut at, Motion::Home, false);
        assert_eq!(heads(&at), [7]);
        let mut at = carets(&[0]);
        motion("local door", &mut at, Motion::Right { word: true }, false);
        assert_eq!(heads(&at), [5]);
    }

    #[test]
    fn a_paste_with_a_line_per_cursor_puts_one_at_each() {
        assert_eq!(paste("a\nb", 2), Edit::InsertEach(vec!["a".into(), "b".into()]));
        assert_eq!(paste("a\nb", 3), Edit::Insert("a\nb".into()));
        let mut text = "1\n2".to_owned();
        let mut at = carets(&[1, 3]);
        edit(&mut text, &mut at, &paste("a\nb", 2));
        assert_eq!(text, "1a\n2b");
        assert_eq!(
            selections("one two", &[Caret { anchor: 4, head: 7 }, Caret { anchor: 0, head: 3 }]),
            ["one", "two"]
        );
    }
}
