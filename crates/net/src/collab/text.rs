//! Collaborative text (ADR 0011 §6): a replicated growable array (RGA), one per script file in a session.
//!
//! Every character has an id ([`CharId`]: who typed it and their Lamport clock then) and is placed just after its
//! left origin, the character that was before it when it was typed. Concurrent inserts at one place are ordered by
//! id, newest first, and an insert skips every character with a newer id than its own, which keeps whatever was typed
//! after those together with them. Deletes leave tombstones, so an insert can always find its origin. The result is
//! the same on every peer whatever order concurrent operations arrive in: only operations travel, never the text.
//!
//! [`TextDoc::local_edit`] turns an edit of the visible text (what the editor's field did this frame) into
//! operations; [`TextDoc::apply`] applies another peer's. [`TextDoc::undo`] and [`TextDoc::redo`] undo the user's own
//! steps only, as operations of their own. [`Anchor`]s place cursors at character ids, so others' edits do not move
//! them off their text.

use std::collections::HashMap;

use crate::codec::{CodecError, Reader, Writer};

/// The longest document, in characters (tombstones included).
pub const MAX_CHARS: usize = 4 << 20;
/// The most characters one insert carries.
pub const MAX_INSERT: usize = 1 << 20;
/// The most ranges one delete carries.
pub const MAX_RANGES: usize = 1 << 16;

/// A character's id: the user who typed it and their clock when they did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharId {
    /// The user's id in the session.
    pub user: u32,
    /// The user's Lamport clock when they typed it.
    pub clock: u32,
}

impl CharId {
    /// Whether `self` is newer than `other`: a later clock, or the same clock and a larger user.
    fn newer(self, other: Self) -> bool {
        (self.clock, self.user) > (other.clock, other.user)
    }

    fn write(self, w: &mut Writer) {
        w.u32(self.user);
        w.u32(self.clock);
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            user: r.u32()?,
            clock: r.u32()?,
        })
    }
}

/// Characters `clock..clock + len` of `user`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharRange {
    /// Their user.
    pub user: u32,
    /// The first's clock.
    pub clock: u32,
    /// How many.
    pub len: u32,
}

/// A change to a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextOp {
    /// `text` typed after `origin` (`None`: at the start); its characters are `first`, then the next clocks of
    /// the same user.
    Insert {
        /// The character before it when it was typed.
        origin: Option<CharId>,
        /// The first character's id.
        first: CharId,
        /// The text.
        text: String,
    },
    /// The characters in `ranges` deleted.
    Delete {
        /// What was deleted.
        ranges: Vec<CharRange>,
    },
}

impl TextOp {
    /// Writes it.
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Insert { origin, first, text } => {
                w.u8(0);
                w.option(*origin, |w, o| o.write(w));
                first.write(w);
                w.str(text);
            }
            Self::Delete { ranges } => {
                w.u8(1);
                w.len_of(ranges.len());
                for range in ranges {
                    w.u32(range.user);
                    w.u32(range.clock);
                    w.u32(range.len);
                }
            }
        }
    }

    /// Reads one written by [`TextOp::write`].
    pub fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(match r.u8()? {
            0 => {
                let origin = r.option(CharId::read)?;
                let first = CharId::read(r)?;
                let text = r.str("inserted text", MAX_INSERT * 4)?.to_owned();
                if text.is_empty() || text.chars().count() > MAX_INSERT {
                    return Err(CodecError::Invalid("an insert of no text, or too much"));
                }
                if first.clock.checked_add(text.chars().count() as u32).is_none() {
                    return Err(CodecError::Overflow);
                }
                Self::Insert { origin, first, text }
            }
            1 => {
                let count = r.count("ranges", MAX_RANGES)?;
                let mut ranges = Vec::with_capacity(count);
                for _ in 0..count {
                    let range = CharRange {
                        user: r.u32()?,
                        clock: r.u32()?,
                        len: r.u32()?,
                    };
                    if range.len == 0 || range.clock.checked_add(range.len).is_none() {
                        return Err(CodecError::Invalid("a delete range that is empty or overflows"));
                    }
                    ranges.push(range);
                }
                Self::Delete { ranges }
            }
            tag => {
                return Err(CodecError::BadTag {
                    what: "a text operation",
                    tag,
                });
            }
        })
    }

    /// The largest clock it uses.
    fn max_clock(&self) -> u32 {
        match self {
            Self::Insert { first, text, .. } => first.clock + text.chars().count() as u32 - 1,
            Self::Delete { .. } => 0,
        }
    }
}

/// A place in the text that stays with its characters: just after the character `after` (`None`: the start).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// The character it follows.
    pub after: Option<CharId>,
}

impl Anchor {
    /// Writes it.
    pub fn write(self, w: &mut Writer) {
        w.option(self.after, |w, a| a.write(w));
    }

    /// Reads one written by [`Anchor::write`].
    pub fn read(r: &mut Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            after: r.option(CharId::read)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Item {
    id: CharId,
    ch: char,
    deleted: bool,
}

/// One of the user's own steps, for undo: what it inserted and what it deleted (with where the text was).
#[derive(Clone, Debug, Default)]
struct Step {
    inserted: Vec<CharId>,
    /// Each deleted run: its characters' ids and text.
    deleted: Vec<(Vec<CharId>, String)>,
}

/// Why an operation could not apply.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// An insert's origin is not in the document.
    #[error("an insert after a character that is not in the document")]
    MissingOrigin,
    /// The document would be too long.
    #[error("the document would be too long")]
    TooLong,
    /// An insert of characters that already exist with other text.
    #[error("an insert of characters that already exist")]
    Duplicate,
}

/// One collaborative document.
#[derive(Clone, Debug)]
pub struct TextDoc {
    items: Vec<Item>,
    /// Where each user's runs of characters are, to find an id fast: (user, clock) to index, rebuilt lazily.
    index: HashMap<CharId, usize>,
    index_valid: bool,
    user: u32,
    clock: u32,
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl TextDoc {
    /// A document holding `text`, its characters made by user `maker` (the host's ids for a file's text when the
    /// session first opened it, so every peer builds the same document from the same text), edited here as `user`.
    pub fn new(text: &str, maker: u32, user: u32) -> Self {
        let items: Vec<Item> = text
            .chars()
            .enumerate()
            .map(|(i, ch)| Item {
                id: CharId {
                    user: maker,
                    clock: i as u32 + 1,
                },
                ch,
                deleted: false,
            })
            .collect();
        let clock = items.len() as u32;
        Self {
            items,
            index: HashMap::new(),
            index_valid: false,
            user,
            clock,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The visible text.
    pub fn text(&self) -> String {
        self.items.iter().filter(|i| !i.deleted).map(|i| i.ch).collect()
    }

    /// How many characters it holds, tombstones included.
    pub fn len_with_tombstones(&self) -> usize {
        self.items.len()
    }

    fn reindex(&mut self) {
        if !self.index_valid {
            self.index.clear();
            self.index.reserve(self.items.len());
            for (i, item) in self.items.iter().enumerate() {
                self.index.insert(item.id, i);
            }
            self.index_valid = true;
        }
    }

    fn position(&mut self, id: CharId) -> Option<usize> {
        self.reindex();
        self.index.get(&id).copied()
    }

    /// The item index of visible character `n` (`None` past the end).
    fn item_of_visible(&self, n: usize) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.deleted)
            .nth(n)
            .map(|(index, _)| index)
    }

    /// The id of the visible character before visible position `n` (`None` at the start).
    fn origin_at(&self, n: usize) -> Option<CharId> {
        if n == 0 {
            return None;
        }
        self.item_of_visible(n - 1).map(|i| self.items[i].id)
    }

    /// Applies another peer's operation. Applying one twice changes nothing.
    pub fn apply(&mut self, op: &TextOp) -> Result<(), TextError> {
        self.clock = self.clock.max(op.max_clock());
        match op {
            TextOp::Insert { origin, first, text } => self.integrate(*origin, *first, text),
            TextOp::Delete { ranges } => {
                for range in ranges {
                    for k in 0..range.len {
                        let id = CharId {
                            user: range.user,
                            clock: range.clock + k,
                        };
                        if let Some(i) = self.position(id) {
                            self.items[i].deleted = true;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn integrate(&mut self, origin: Option<CharId>, first: CharId, text: &str) -> Result<(), TextError> {
        let count = text.chars().count();
        if self.items.len() + count > MAX_CHARS {
            return Err(TextError::TooLong);
        }
        if let Some(i) = self.position(first) {
            // Applied already (the same characters): nothing to do.
            return if self.items[i].ch == text.chars().next().unwrap_or_default() {
                Ok(())
            } else {
                Err(TextError::Duplicate)
            };
        }
        let mut at = match origin {
            None => 0,
            Some(origin) => self.position(origin).ok_or(TextError::MissingOrigin)? + 1,
        };
        for (k, ch) in text.chars().enumerate() {
            let id = CharId {
                user: first.user,
                clock: first.clock + k as u32,
            };
            if k == 0 {
                // Past every newer character at this place: they were typed after (or concurrently, and win).
                while at < self.items.len() && self.items[at].id.newer(id) {
                    at += 1;
                }
            }
            // The rest follow each other: each one's origin is the one before, and nothing newer can be between them
            // yet but what was typed after them, which comes after them.
            else {
                while at < self.items.len() && self.items[at].id.newer(id) {
                    at += 1;
                }
            }
            self.items.insert(at, Item { id, ch, deleted: false });
            at += 1;
        }
        self.index_valid = false;
        Ok(())
    }

    fn next_ids(&mut self, count: usize) -> CharId {
        let first = CharId {
            user: self.user,
            clock: self.clock + 1,
        };
        self.clock += count as u32;
        first
    }

    /// Turns `new`, the visible text after the user's edit, into operations, applies them, and notes them as one
    /// step for undo (`merge`: joined to the last step, as typing is).
    pub fn local_edit(&mut self, new: &str, merge: bool) -> Vec<TextOp> {
        let old: Vec<char> = self.text().chars().collect();
        let new: Vec<char> = new.chars().collect();
        let edits = diff(&old, &new);
        let mut ops = Vec::new();
        let mut step = Step::default();
        // From the end, so earlier visible positions stay where they were.
        for edit in edits.into_iter().rev() {
            if edit.delete > 0 {
                let mut ids = Vec::with_capacity(edit.delete);
                let mut deleted = String::new();
                for n in edit.at..edit.at + edit.delete {
                    if let Some(i) = self.item_of_visible(n) {
                        ids.push(self.items[i].id);
                        deleted.push(self.items[i].ch);
                    }
                }
                for &id in &ids {
                    if let Some(i) = self.position(id) {
                        self.items[i].deleted = true;
                    }
                }
                ops.push(TextOp::Delete { ranges: ranges(&ids) });
                step.deleted.push((ids, deleted));
            }
            if !edit.insert.is_empty() {
                let origin = self.origin_at(edit.at);
                let text: String = edit.insert.iter().collect();
                let first = self.next_ids(edit.insert.len());
                // The local document always has the origin.
                let _ = self.integrate(origin, first, &text);
                step.inserted.extend((0..edit.insert.len() as u32).map(|k| CharId {
                    user: first.user,
                    clock: first.clock + k,
                }));
                ops.push(TextOp::Insert { origin, first, text });
            }
        }
        if !ops.is_empty() {
            self.redo.clear();
            match self.undo.last_mut() {
                Some(last) if merge => {
                    last.inserted.extend(step.inserted);
                    last.deleted.extend(step.deleted);
                }
                _ => self.undo.push(step),
            }
        }
        ops
    }

    /// Whether there is a step of the user's own to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the user's last step: its insertions deleted (those still there), its deletions typed again where
    /// they were. Others' edits stay. Returns the operations and the visible position the cursor goes to.
    pub fn undo(&mut self) -> Option<(Vec<TextOp>, usize)> {
        let step = self.undo.pop()?;
        let (ops, inverse, at) = self.invert(&step);
        self.redo.push(inverse);
        Some((ops, at))
    }

    /// Redoes the last step undone.
    pub fn redo(&mut self) -> Option<(Vec<TextOp>, usize)> {
        let step = self.redo.pop()?;
        let (ops, inverse, at) = self.invert(&step);
        self.undo.push(inverse);
        Some((ops, at))
    }

    /// The operations that take `step` back, the step that takes those back, and where the cursor goes.
    fn invert(&mut self, step: &Step) -> (Vec<TextOp>, Step, usize) {
        let mut ops = Vec::new();
        let mut inverse = Step::default();
        let mut at = None;
        // Its insertions go.
        let live: Vec<CharId> = step
            .inserted
            .iter()
            .copied()
            .filter(|&id| self.position(id).is_some_and(|i| !self.items[i].deleted))
            .collect();
        if !live.is_empty() {
            let mut text = String::new();
            for &id in &live {
                if let Some(i) = self.position(id) {
                    text.push(self.items[i].ch);
                    self.items[i].deleted = true;
                    at.get_or_insert(self.visible_before(i));
                }
            }
            ops.push(TextOp::Delete { ranges: ranges(&live) });
            inverse.deleted.push((live, text));
        }
        // Its deletions come back, after the last character deleted (a tombstone, so they land where they were).
        for (ids, text) in step.deleted.iter().rev() {
            let Some(&last) = ids.last() else {
                continue;
            };
            if self.position(last).is_none() {
                continue;
            }
            let count = text.chars().count();
            let first = self.next_ids(count);
            let _ = self.integrate(Some(last), first, text);
            let new: Vec<CharId> = (0..count as u32)
                .map(|k| CharId {
                    user: first.user,
                    clock: first.clock + k,
                })
                .collect();
            if let Some(i) = self.position(first) {
                at.get_or_insert(self.visible_before(i) + count);
            }
            inverse.inserted.extend(new);
            ops.push(TextOp::Insert {
                origin: Some(last),
                first,
                text: text.clone(),
            });
        }
        (ops, inverse, at.unwrap_or(0))
    }

    /// How many visible characters are before item `i`.
    fn visible_before(&self, i: usize) -> usize {
        self.items[..i].iter().filter(|item| !item.deleted).count()
    }

    /// The anchor at visible position `n` (characters before it), to send as a cursor.
    pub fn anchor_at(&self, n: usize) -> Anchor {
        Anchor {
            after: self.origin_at(n),
        }
    }

    /// The visible position `anchor` stands at now: after its character (wherever that went; a deleted one counts
    /// as where it was).
    pub fn position_of(&mut self, anchor: Anchor) -> usize {
        match anchor.after {
            None => 0,
            Some(id) => match self.position(id) {
                Some(i) => self.visible_before(i) + usize::from(!self.items[i].deleted),
                None => 0,
            },
        }
    }

    /// Moves a visible position `n` over `ops` applied since: where the same place is now. For the user's own
    /// cursor when others' edits come in.
    pub fn moved(&mut self, anchor: Anchor) -> usize {
        self.position_of(anchor)
    }

    /// The document's whole state, tombstones included, for a peer that joins while it is edited.
    pub fn write_state(&self, w: &mut Writer) {
        // Runs of consecutive ids of one user with the same deleted flag.
        let mut runs: Vec<(CharId, bool, String)> = Vec::new();
        for item in &self.items {
            match runs.last_mut() {
                Some((first, deleted, text))
                    if *deleted == item.deleted
                        && first.user == item.id.user
                        && first.clock + text.chars().count() as u32 == item.id.clock =>
                {
                    text.push(item.ch);
                }
                _ => runs.push((item.id, item.deleted, item.ch.to_string())),
            }
        }
        w.u32(self.clock);
        w.len_of(runs.len());
        for (first, deleted, text) in runs {
            first.write(w);
            w.bool(deleted);
            w.str(&text);
        }
    }

    /// A document from [`TextDoc::write_state`]'s bytes, edited here as `user`.
    pub fn read_state(r: &mut Reader<'_>, user: u32) -> Result<Self, CodecError> {
        let clock = r.u32()?;
        let count = r.count("runs", MAX_CHARS)?;
        let mut items = Vec::new();
        for _ in 0..count {
            let first = CharId::read(r)?;
            let deleted = r.bool()?;
            let text = r.str("a run", MAX_CHARS * 4)?;
            for (k, ch) in text.chars().enumerate() {
                if items.len() >= MAX_CHARS {
                    return Err(CodecError::Invalid("a document too long"));
                }
                items.push(Item {
                    id: CharId {
                        user: first.user,
                        clock: first.clock.checked_add(k as u32).ok_or(CodecError::Overflow)?,
                    },
                    ch,
                    deleted,
                });
            }
        }
        Ok(Self {
            items,
            index: HashMap::new(),
            index_valid: false,
            user,
            clock,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
}

/// Ids as ranges of consecutive clocks per user.
fn ranges(ids: &[CharId]) -> Vec<CharRange> {
    let mut out: Vec<CharRange> = Vec::new();
    for id in ids {
        match out.last_mut() {
            Some(range) if range.user == id.user && range.clock + range.len == id.clock => range.len += 1,
            _ => out.push(CharRange {
                user: id.user,
                clock: id.clock,
                len: 1,
            }),
        }
    }
    out
}

/// One edit of a diff: at visible position `at` of the old text, `delete` characters replaced by `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    at: usize,
    delete: usize,
    insert: Vec<char>,
}

/// The largest middle (after the common start and end are cut) a full diff is run on; past it, one replace.
const DIFF_LIMIT: usize = 4000;

/// The edits from `old` to `new`, in order: the common start and end kept, and what is between them diffed (Myers)
/// when it is small, so several separate edits in one frame (several cursors) stay separate.
fn diff(old: &[char], new: &[char]) -> Vec<Edit> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.is_empty() || b.is_empty() || a.len() + b.len() > DIFF_LIMIT {
        return vec![Edit {
            at: prefix,
            delete: a.len(),
            insert: b.to_vec(),
        }];
    }
    myers(a, b)
        .into_iter()
        .map(|mut edit| {
            edit.at += prefix;
            edit
        })
        .collect()
}

/// Myers' O(ND) diff of `a` and `b`, as edits on `a`.
fn myers(a: &[char], b: &[char]) -> Vec<Edit> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let offset = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'outer: for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                break 'outer;
            }
            k += 2;
        }
    }
    // Walk back through the trace to the moves.
    let mut x = n;
    let mut y = m;
    let mut steps: Vec<(isize, isize, isize, isize)> = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let index = (k + offset) as usize;
        let prev_k = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            steps.push((prev_x, prev_y, x, y));
        }
        x = prev_x;
        y = prev_y;
    }
    steps.reverse();
    // Each step is one deletion (x moves) or one insertion (y moves); merge neighbours into edits.
    let mut edits: Vec<Edit> = Vec::new();
    for (px, py, x, _y) in steps {
        let (at, delete, insert) = if x > px {
            (px as usize, 1, Vec::new())
        } else {
            (px as usize, 0, vec![b[py as usize]])
        };
        match edits.last_mut() {
            Some(last) if last.at + last.delete == at => {
                last.delete += delete;
                last.insert.extend(insert);
            }
            _ => edits.push(Edit { at, delete, insert }),
        }
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(text: &str, user: u32) -> TextDoc {
        TextDoc::new(text, 1, user)
    }

    #[test]
    fn concurrent_typing_at_one_place_converges_and_keeps_both() {
        let mut a = peer("hello world", 2);
        let mut b = peer("hello world", 3);
        let ops_a = a.local_edit("hello brave world", false);
        let ops_b = b.local_edit("hello wide world", false);
        for op in &ops_b {
            a.apply(op).unwrap();
        }
        for op in &ops_a {
            b.apply(op).unwrap();
        }
        assert_eq!(a.text(), b.text());
        assert!(a.text().contains("brave") && a.text().contains("wide"), "{}", a.text());
    }

    #[test]
    fn interleaved_edits_in_any_order_converge() {
        let base = "fn main()\n  print(1)\nend\n";
        let mut docs: Vec<TextDoc> = (2..5).map(|u| peer(base, u)).collect();
        let edits = [
            "fn main()\n  print(1)\n  print(2)\nend\n",
            "local x = 1\nfn main()\n  print(1)\nend\n",
            "fn main()\n  print(10)\nend\n",
        ];
        let ops: Vec<Vec<TextOp>> = docs
            .iter_mut()
            .zip(edits)
            .map(|(doc, edit)| doc.local_edit(edit, false))
            .collect();
        for (i, doc) in docs.iter_mut().enumerate() {
            // Each applies the others' in a different order.
            let mut order: Vec<usize> = (0..3).filter(|&j| j != i).collect();
            if i % 2 == 1 {
                order.reverse();
            }
            for j in order {
                for op in &ops[j] {
                    doc.apply(op).unwrap();
                }
            }
        }
        assert_eq!(docs[0].text(), docs[1].text());
        assert_eq!(docs[1].text(), docs[2].text());
        let text = docs[0].text();
        assert!(
            text.contains("print(2)") && text.contains("local x") && text.contains("print(10)"),
            "{text}"
        );
    }

    #[test]
    fn a_delete_and_an_insert_inside_it_both_count() {
        let mut a = peer("abcdef", 2);
        let mut b = peer("abcdef", 3);
        let del = a.local_edit("af", false);
        let ins = b.local_edit("abcXdef", false);
        for op in &ins {
            a.apply(op).unwrap();
        }
        for op in &del {
            b.apply(op).unwrap();
        }
        assert_eq!(a.text(), "aXf");
        assert_eq!(b.text(), "aXf");
    }

    #[test]
    fn undo_takes_back_only_your_own_typing() {
        let mut a = peer("one\n", 2);
        let mut b = peer("one\n", 3);
        let mine = a.local_edit("one\ntwo\n", false);
        for op in &mine {
            b.apply(op).unwrap();
        }
        let theirs = b.local_edit("zero\none\ntwo\n", false);
        for op in &theirs {
            a.apply(op).unwrap();
        }
        let (undo, _) = a.undo().unwrap();
        assert_eq!(a.text(), "zero\none\n", "their line stays");
        for op in &undo {
            b.apply(op).unwrap();
        }
        assert_eq!(b.text(), "zero\none\n");
        let (redo, _) = a.redo().unwrap();
        for op in &redo {
            b.apply(op).unwrap();
        }
        assert_eq!(a.text(), "zero\none\ntwo\n");
        assert_eq!(b.text(), a.text());
        // Undoing a delete types it back where it was.
        let del = a.local_edit("zero\ntwo\n", false);
        for op in &del {
            b.apply(op).unwrap();
        }
        let (back, _) = a.undo().unwrap();
        for op in &back {
            b.apply(op).unwrap();
        }
        assert_eq!(a.text(), "zero\none\ntwo\n");
        assert_eq!(b.text(), a.text());
    }

    #[test]
    fn anchors_stay_with_their_text() {
        let mut a = peer("local x = 1", 2);
        let mut b = peer("local x = 1", 3);
        // B's cursor after "x".
        let cursor = b.anchor_at(7);
        let ops = a.local_edit("-- note\nlocal x = 1", false);
        for op in &ops {
            b.apply(op).unwrap();
        }
        assert_eq!(&b.text()[..b.position_of(cursor)], "-- note\nlocal x");
        // Deleting the character it follows leaves it where that was.
        let ops = a.local_edit("-- note\nlocal  = 1", false);
        for op in &ops {
            b.apply(op).unwrap();
        }
        assert_eq!(&b.text()[..b.position_of(cursor)], "-- note\nlocal ");
    }

    #[test]
    fn a_late_joiner_gets_the_same_document() {
        let mut a = peer("abc", 2);
        a.local_edit("aXbc", false);
        a.local_edit("aXc", false);
        let mut w = Writer::new();
        a.write_state(&mut w);
        let bytes = w.finish();
        let mut late = TextDoc::read_state(&mut Reader::new(&bytes), 3).unwrap();
        assert_eq!(late.text(), "aXc");
        // An edit made against the old state still lands (its origin is a tombstone the joiner has).
        let op = TextOp::Insert {
            origin: Some(CharId { user: 1, clock: 2 }),
            first: CharId { user: 9, clock: 50 },
            text: "!".into(),
        };
        late.apply(&op).unwrap();
        a.apply(&op).unwrap();
        assert_eq!(late.text(), a.text());
    }

    #[test]
    fn several_separate_edits_in_one_frame_stay_separate() {
        let mut a = peer("aaa bbb ccc", 2);
        let ops = a.local_edit("aaa! bbb ccc!", false);
        assert_eq!(ops.len(), 2, "{ops:?}");
        assert_eq!(a.text(), "aaa! bbb ccc!");
    }

    #[test]
    fn operations_round_trip_and_bad_ones_are_refused() {
        let ops = [
            TextOp::Insert {
                origin: None,
                first: CharId { user: 2, clock: 1 },
                text: "héllo".into(),
            },
            TextOp::Delete {
                ranges: vec![CharRange {
                    user: 2,
                    clock: 1,
                    len: 3,
                }],
            },
        ];
        for op in ops {
            let mut w = Writer::new();
            op.write(&mut w);
            let bytes = w.finish();
            assert_eq!(TextOp::read(&mut Reader::new(&bytes)).unwrap(), op);
        }
        let mut doc = peer("x", 2);
        let orphan = TextOp::Insert {
            origin: Some(CharId { user: 7, clock: 7 }),
            first: CharId { user: 3, clock: 9 },
            text: "y".into(),
        };
        assert_eq!(doc.apply(&orphan), Err(TextError::MissingOrigin));
    }
}
