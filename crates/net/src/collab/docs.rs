//! Script text in a session (ADR 0011 §6): one [`TextDoc`] per script file, the host making each and relaying edits.
//!
//! - **Making a document.** Only the host makes one, from the file's text as it has it (characters by user 0,
//!   clocks 1..n), and sends its state to everyone; a participant asks for one with [`kind::OPEN`] when it opens a
//!   script that has none yet. A participant that joins later gets every document's state right after the project.
//! - **Edits** are operations, never the text: a participant applies its own at once and sends them; the host
//!   checks them (the participant's own ids, a document that exists, origins it has), applies them and passes them
//!   on in its order. The CRDT makes the order of concurrent edits not matter, so nobody waits for an
//!   acknowledgement. An edit that cannot apply gets the sender the document's state again.

use std::collections::{BTreeMap, HashSet};

use super::protocol::is_safe_member_path;
use super::text::{TextDoc, TextOp};
use crate::codec::{CodecError, Reader, Writer};
use crate::identity::UserId;

/// The message kinds of script text.
pub mod kind {
    /// A participant asks for a document.
    pub const OPEN: u8 = 30;
    /// A document's whole state.
    pub const STATE: u8 = 31;
    /// A participant's edits, to the host.
    pub const EDIT: u8 = 32;
    /// Someone's edits, from the host.
    pub const EDITS: u8 = 33;
}

/// The most operations one edit message carries.
const MAX_OPS: usize = 4096;

/// Whether `member` is a script file a document may be made for.
pub fn is_script_member(member: &str) -> bool {
    is_safe_member_path(member) && member.ends_with(".luau")
}

fn read_member(r: &mut Reader<'_>) -> Result<String, CodecError> {
    let member = r.str("a script's path", super::protocol::MAX_PATH)?;
    if !is_script_member(member) {
        return Err(CodecError::Invalid("a script path that is not one"));
    }
    Ok(member.to_owned())
}

fn ops_body(member: &str, ops: &[TextOp]) -> Vec<u8> {
    let mut w = Writer::new();
    w.str(member);
    w.len_of(ops.len());
    for op in ops {
        op.write(&mut w);
    }
    w.finish()
}

fn read_ops(r: &mut Reader<'_>) -> Result<Vec<TextOp>, CodecError> {
    let count = r.count("text operations", MAX_OPS)?;
    (0..count).map(|_| TextOp::read(r)).collect()
}

fn state_body(member: &str, doc: &TextDoc) -> Vec<u8> {
    let mut w = Writer::new();
    w.str(member);
    doc.write_state(&mut w);
    w.finish()
}

/// What the host has to send after a step of text sync.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DocOut {
    /// To everyone but `except`: the kind and bytes.
    pub all: Vec<(u8, Vec<u8>, Option<UserId>)>,
    /// To one user.
    pub one: Vec<(UserId, u8, Vec<u8>)>,
    /// The documents whose text changed (write them into the project).
    pub changed: Vec<String>,
}

/// The host's documents.
#[derive(Debug, Default)]
pub struct HostDocs {
    docs: BTreeMap<String, TextDoc>,
}

impl HostDocs {
    /// The document of `member`, if there is one.
    pub fn get(&self, member: &str) -> Option<&TextDoc> {
        self.docs.get(member)
    }

    /// The document of `member`, to change.
    pub fn get_mut(&mut self, member: &str) -> Option<&mut TextDoc> {
        self.docs.get_mut(member)
    }

    /// The members that have documents.
    pub fn members(&self) -> impl Iterator<Item = &str> {
        self.docs.keys().map(String::as_str)
    }

    /// Makes `member`'s document from `text` unless it has one, and queues its state for everyone. Returns whether
    /// it was made.
    pub fn ensure(&mut self, member: &str, text: &str, out: &mut DocOut) -> bool {
        if self.docs.contains_key(member) || !is_script_member(member) {
            return false;
        }
        let doc = TextDoc::new(text, 0, UserId::HOST.0);
        out.all.push((kind::STATE, state_body(member, &doc), None));
        self.docs.insert(member.to_owned(), doc);
        true
    }

    /// Every document's state for `to`, a participant who just joined.
    pub fn send_all(&self, to: UserId, out: &mut DocOut) {
        for (member, doc) in &self.docs {
            out.one.push((to, kind::STATE, state_body(member, doc)));
        }
    }

    /// The host's own edit of `member`: its text is now `text`. Merged into the last undo step with `merge`.
    pub fn local_edit(&mut self, member: &str, text: &str, merge: bool, out: &mut DocOut) {
        let Some(doc) = self.docs.get_mut(member) else {
            return;
        };
        let ops = doc.local_edit(text, merge);
        if !ops.is_empty() {
            out.all.push((kind::EDITS, ops_body(member, &ops), None));
        }
    }

    /// The host's undo (or redo) in `member`: the operations go to everyone. Returns the new text and where the
    /// cursor goes.
    pub fn undo(&mut self, member: &str, redo: bool, out: &mut DocOut) -> Option<(String, usize)> {
        let doc = self.docs.get_mut(member)?;
        let (ops, at) = if redo { doc.redo()? } else { doc.undo()? };
        if !ops.is_empty() {
            out.all.push((kind::EDITS, ops_body(member, &ops), None));
        }
        Some((doc.text(), at))
    }

    /// A message of `kind` from participant `from`. An error is a message that cannot be read: drop the peer.
    pub fn receive(
        &mut self,
        from: UserId,
        kind: u8,
        bytes: &[u8],
        file_text: &mut dyn FnMut(&str) -> String,
        out: &mut DocOut,
    ) -> Result<(), CodecError> {
        let mut r = Reader::new(bytes);
        match kind {
            kind::OPEN => {
                let member = read_member(&mut r)?;
                r.finish()?;
                if !self.ensure(&member, &file_text(&member), out) {
                    // It has one: the asker gets its state (they may have missed it).
                    if let Some(doc) = self.docs.get(&member) {
                        out.one.push((from, kind::STATE, state_body(&member, doc)));
                    }
                }
            }
            kind::EDIT => {
                let member = read_member(&mut r)?;
                let ops = read_ops(&mut r)?;
                r.finish()?;
                let Some(doc) = self.docs.get_mut(&member) else {
                    return Ok(());
                };
                let mut applied = Vec::with_capacity(ops.len());
                let mut broken = false;
                for op in ops {
                    if let TextOp::Insert { first, .. } = &op
                        && first.user != from.0
                    {
                        // Characters in someone else's name.
                        broken = true;
                        break;
                    }
                    if doc.apply(&op).is_err() {
                        broken = true;
                        break;
                    }
                    applied.push(op);
                }
                if !applied.is_empty() {
                    out.all.push((kind::EDITS, ops_body(&member, &applied), Some(from)));
                    out.changed.push(member.clone());
                }
                if broken {
                    log::warn!("team create: an edit of {member} from {from} could not apply; sending the document");
                    out.one.push((from, kind::STATE, state_body(&member, doc)));
                }
            }
            _ => {
                return Err(CodecError::BadTag {
                    what: "a text message",
                    tag: kind,
                });
            }
        }
        Ok(())
    }
}

/// What a participant's step of text sync did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DocEvents {
    /// Documents that arrived or were replaced: their text is the document's now.
    pub replaced: Vec<String>,
    /// Documents others edited.
    pub edited: Vec<String>,
}

/// A participant's documents.
#[derive(Debug)]
pub struct ClientDocs {
    me: UserId,
    docs: BTreeMap<String, TextDoc>,
    asked: HashSet<String>,
}

impl ClientDocs {
    /// No documents yet, for participant `me`.
    pub fn new(me: UserId) -> Self {
        Self {
            me,
            docs: BTreeMap::new(),
            asked: HashSet::new(),
        }
    }

    /// The document of `member`, if there is one.
    pub fn get(&self, member: &str) -> Option<&TextDoc> {
        self.docs.get(member)
    }

    /// The document of `member`, to change.
    pub fn get_mut(&mut self, member: &str) -> Option<&mut TextDoc> {
        self.docs.get_mut(member)
    }

    /// The members that have documents.
    pub fn members(&self) -> impl Iterator<Item = &str> {
        self.docs.keys().map(String::as_str)
    }

    /// The message asking for `member`'s document, unless it has one or asked already.
    pub fn open(&mut self, member: &str) -> Option<(u8, Vec<u8>)> {
        if self.docs.contains_key(member) || !is_script_member(member) || !self.asked.insert(member.to_owned()) {
            return None;
        }
        let mut w = Writer::new();
        w.str(member);
        Some((kind::OPEN, w.finish()))
    }

    /// The participant's own edit of `member`: its text is now `text`. Returns the message for the host, if it
    /// changed anything.
    pub fn local_edit(&mut self, member: &str, text: &str, merge: bool) -> Option<(u8, Vec<u8>)> {
        let doc = self.docs.get_mut(member)?;
        let ops = doc.local_edit(text, merge);
        (!ops.is_empty()).then(|| (kind::EDIT, ops_body(member, &ops)))
    }

    /// The participant's undo (or redo) in `member`: the message for the host, the new text, and the cursor.
    pub fn undo(&mut self, member: &str, redo: bool) -> Option<((u8, Vec<u8>), String, usize)> {
        let doc = self.docs.get_mut(member)?;
        let (ops, at) = if redo { doc.redo()? } else { doc.undo()? };
        Some(((kind::EDIT, ops_body(member, &ops)), doc.text(), at))
    }

    /// A message of `kind` from the host. An error is a message that cannot be read: the session is broken.
    pub fn receive(&mut self, kind: u8, bytes: &[u8]) -> Result<DocEvents, CodecError> {
        let mut r = Reader::new(bytes);
        let mut events = DocEvents::default();
        match kind {
            kind::STATE => {
                let member = read_member(&mut r)?;
                let doc = TextDoc::read_state(&mut r, self.me.0)?;
                r.finish()?;
                self.asked.remove(&member);
                self.docs.insert(member.clone(), doc);
                events.replaced.push(member);
            }
            kind::EDITS => {
                let member = read_member(&mut r)?;
                let ops = read_ops(&mut r)?;
                r.finish()?;
                if let Some(doc) = self.docs.get_mut(&member) {
                    for op in &ops {
                        if doc.apply(op).is_err() {
                            return Err(CodecError::Invalid("an edit the document cannot take"));
                        }
                    }
                    events.edited.push(member);
                }
            }
            _ => {
                return Err(CodecError::BadTag {
                    what: "a text message",
                    tag: kind,
                });
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOOR: &str = "Scripts/Door.luau";

    /// Delivers `out` from the host to the participants (`users[i]` is participant `i`).
    fn deliver(out: DocOut, clients: &mut [&mut ClientDocs], users: &[UserId]) {
        for (kind, bytes, except) in out.all {
            for (client, user) in clients.iter_mut().zip(users) {
                if Some(*user) != except {
                    client.receive(kind, &bytes).unwrap();
                }
            }
        }
        for (to, kind, bytes) in out.one {
            for (client, user) in clients.iter_mut().zip(users) {
                if *user == to {
                    client.receive(kind, &bytes).unwrap();
                }
            }
        }
    }

    #[test]
    fn two_users_typing_at_once_end_with_the_same_text_and_nothing_lost() {
        let mut host = HostDocs::default();
        let mut a = ClientDocs::new(UserId(2));
        let mut b = ClientDocs::new(UserId(3));
        let users = [UserId(2), UserId(3)];
        let mut out = DocOut::default();
        host.ensure(DOOR, "local open = false\n", &mut out);
        deliver(out, &mut [&mut a, &mut b], &users);
        // Both type at once, at the same place and elsewhere.
        let edit_a = a.local_edit(DOOR, "-- A\nlocal open = false\n", false).unwrap();
        let edit_b = b
            .local_edit(DOOR, "local open = false\nprint(open) -- B\n", false)
            .unwrap();
        for (from, (kind, bytes)) in [(UserId(2), edit_a), (UserId(3), edit_b)] {
            let mut out = DocOut::default();
            host.receive(from, kind, &bytes, &mut |_| String::new(), &mut out)
                .unwrap();
            assert_eq!(out.changed, [DOOR]);
            deliver(out, &mut [&mut a, &mut b], &users);
        }
        let text = host.get(DOOR).unwrap().text();
        assert_eq!(a.get(DOOR).unwrap().text(), text);
        assert_eq!(b.get(DOOR).unwrap().text(), text);
        assert!(text.contains("-- A") && text.contains("-- B"), "{text}");
    }

    #[test]
    fn a_late_joiner_gets_every_document_and_forged_edits_are_refused() {
        let mut host = HostDocs::default();
        let mut out = DocOut::default();
        host.ensure(DOOR, "x", &mut out);
        host.local_edit(DOOR, "xy", false, &mut out);
        let mut late = ClientDocs::new(UserId(4));
        let mut out = DocOut::default();
        host.send_all(UserId(4), &mut out);
        deliver(out, &mut [&mut late], &[UserId(4)]);
        assert_eq!(late.get(DOOR).unwrap().text(), "xy");
        // An insert in user 2's name, sent by user 4: refused, and the sender gets the document back.
        let forged = ops_body(
            DOOR,
            &[TextOp::Insert {
                origin: None,
                first: super::super::text::CharId { user: 2, clock: 99 },
                text: "!".into(),
            }],
        );
        let mut out = DocOut::default();
        host.receive(UserId(4), kind::EDIT, &forged, &mut |_| String::new(), &mut out)
            .unwrap();
        assert!(out.all.is_empty());
        assert_eq!(out.one.len(), 1);
        assert_eq!(host.get(DOOR).unwrap().text(), "xy");
        // A path that is not a script is not read.
        let mut w = Writer::new();
        w.str("../secret.txt");
        let mut out = DocOut::default();
        assert!(
            host.receive(UserId(4), kind::OPEN, &w.finish(), &mut |_| String::new(), &mut out)
                .is_err()
        );
    }
}
