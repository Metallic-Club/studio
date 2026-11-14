//! The script editor's side of collaborative text (ADR 0011 §6, #631): the editor's texts compared with the
//! session's documents each frame, others' edits put into the texts with the user's cursor kept where it was, and
//! cursors as presence.
//!
//! The code field stays an ordinary text field: what it typed is found by comparing its text with the document's
//! (so every way it edits, completion and find and replace included, is covered), and what others typed replaces
//! its text with the document's, the cursor moved by an anchor (the character it follows), so it stays on the same
//! code. Undo and redo in a collaborative file are the document's: they undo the user's own edits only.

use std::time::{Duration, Instant};

use metallic_engine::metallic_net::codec::{Reader, Writer};
use metallic_engine::metallic_net::collab::presence::Presence;
use metallic_engine::metallic_net::collab::text::{Anchor, TextDoc};
use metallic_engine::metallic_net::identity::{User, UserId};

use crate::code::RemoteCaret;
use crate::editor::EditorState;

/// Edits of one file closer together than this are one undo step (typing).
pub const MERGE_WITHIN: Duration = Duration::from_millis(600);
/// How long after a user typed they show as typing.
pub const TYPING: Duration = Duration::from_millis(1200);

/// The session's documents, whoever keeps them.
pub trait Docs {
    /// The document of `member`.
    fn doc(&self, member: &str) -> Option<&TextDoc>;
    /// The document of `member`, to change.
    fn doc_mut(&mut self, member: &str) -> Option<&mut TextDoc>;
    /// Every member with a document.
    fn members(&self) -> Vec<String>;
}

impl Docs for metallic_engine::metallic_net::collab::docs::HostDocs {
    fn doc(&self, member: &str) -> Option<&TextDoc> {
        self.get(member)
    }
    fn doc_mut(&mut self, member: &str) -> Option<&mut TextDoc> {
        self.get_mut(member)
    }
    fn members(&self) -> Vec<String> {
        metallic_engine::metallic_net::collab::docs::HostDocs::members(self)
            .map(str::to_owned)
            .collect()
    }
}

impl Docs for metallic_engine::metallic_net::collab::docs::ClientDocs {
    fn doc(&self, member: &str) -> Option<&TextDoc> {
        self.get(member)
    }
    fn doc_mut(&mut self, member: &str) -> Option<&mut TextDoc> {
        self.get_mut(member)
    }
    fn members(&self) -> Vec<String> {
        metallic_engine::metallic_net::collab::docs::ClientDocs::members(self)
            .map(str::to_owned)
            .collect()
    }
}

/// The files whose text the user changed since the documents last had it: each with its text now and whether it
/// joins the last undo step.
pub fn local_changes(editor: &mut EditorState, docs: &impl Docs) -> Vec<(String, String, bool)> {
    let mut changes = Vec::new();
    let now = Instant::now();
    for member in docs.members() {
        let Some(doc) = docs.doc(&member) else {
            continue;
        };
        let text = editor.scripts.text(&mut editor.project, &member).clone();
        if text != doc.text() {
            let merge = editor
                .scripts
                .collab
                .typed
                .get(&member)
                .is_some_and(|at| now.duration_since(*at) < MERGE_WITHIN);
            changes.push((member.clone(), text, merge));
            editor.scripts.collab.typed.insert(member, now);
        }
    }
    changes
}

/// The focused field's selection as anchors in its document, before others' edits apply.
pub fn anchors(editor: &EditorState, docs: &impl Docs) -> Option<(String, Anchor, Anchor)> {
    let (member, tail, head) = editor.scripts.collab.cursor.clone()?;
    let doc = docs.doc(&member)?;
    Some((member, doc.anchor_at(tail), doc.anchor_at(head)))
}

/// Puts `member`'s document's text into the editor; the focused field's cursor, if it is in that file, kept on the
/// same code by `anchors`.
pub fn apply(editor: &mut EditorState, docs: &mut impl Docs, member: &str, anchors: Option<&(String, Anchor, Anchor)>) {
    let Some(doc) = docs.doc_mut(member) else {
        return;
    };
    let text = doc.text();
    if let Some((file, tail, head)) = anchors
        && file == member
    {
        let (tail, head) = (doc.position_of(*tail), doc.position_of(*head));
        editor.scripts.collab.moves.insert(member.to_owned(), (tail, head));
    }
    if *editor.scripts.text(&mut editor.project, member) != text {
        editor.scripts.set_from_session(&mut editor.project, member, text);
    }
}

/// The documents' members, for the code fields to know whose undo is the session's.
pub fn publish(editor: &mut EditorState, docs: &impl Docs) {
    editor.scripts.collab.members = docs.members().into_iter().collect();
}

/// This user's cursor as presence: the file, the selection's anchors, and whether they are typing.
pub fn cursor_bytes(editor: &EditorState, docs: &impl Docs) -> Option<Vec<u8>> {
    let (member, tail, head) = anchors(editor, docs)?;
    let typing = editor
        .scripts
        .collab
        .typed
        .get(&member)
        .is_some_and(|at| at.elapsed() < TYPING);
    let mut w = Writer::new();
    w.str(&member);
    tail.write(&mut w);
    head.write(&mut w);
    w.bool(typing);
    Some(w.finish())
}

/// The others' cursors, from their presence, placed in the documents as they are now.
pub fn show_carets(
    editor: &mut EditorState,
    docs: &mut impl Docs,
    users: &[User],
    presences: &[(UserId, &Presence)],
    me: UserId,
) {
    let mut remote: std::collections::HashMap<String, Vec<RemoteCaret>> = Default::default();
    for (id, presence) in presences {
        if *id == me {
            continue;
        }
        let (Some(user), Some((_, bytes))) = (users.iter().find(|u| u.id == *id), &presence.cursor) else {
            continue;
        };
        let mut r = Reader::new(bytes);
        let Ok(member) = r.str("a file", 512).map(str::to_owned) else {
            continue;
        };
        let (Ok(tail), Ok(head), Ok(typing)) = (Anchor::read(&mut r), Anchor::read(&mut r), r.bool()) else {
            continue;
        };
        let Some(doc) = docs.doc_mut(&member) else {
            continue;
        };
        let caret = RemoteCaret {
            name: user.identity.name.clone(),
            color: user.identity.color,
            head: doc.position_of(head),
            tail: doc.position_of(tail),
            typing,
        };
        remote.entry(member).or_default().push(caret);
    }
    editor.scripts.collab.remote = remote;
}
