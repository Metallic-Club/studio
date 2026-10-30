//! Edit-only authoritative command kernel shared by a future Studio document and edit server.
//!
//! [`EditAuthority`] owns its [`World`]. It preflights an entire batch on an independent
//! [`World::deep_clone`] and then replays into the original World under exclusive ownership.
//! Rejected input cannot change that World's data, cursor or original subscriptions. Staging
//! costs **O(world)** per batch; this foundation is not a large-document performance claim.
//!
//! Successful replay preserves existing per-operation callbacks and journal behavior. It does
//! not replace the live World or its signals. Callback panics are not rolled back; a live replay
//! panic or unexpected replay error leaves the authority terminally poisoned until reload.
//! This module does not run scripts/physics, authenticate editors, time script leases, perform
//! optimistic UI/undo, transport records or durably save a session. [`EditReplica`] accepts only
//! a trusted authority's ordered records, not arbitrary user submissions.

mod state;
mod types;

pub use types::*;

use crate::{InstanceId, World};
use state::{Bindings, Document, measure, validate_limits};
use std::collections::{BTreeMap, VecDeque};

struct Client {
    access: EditAccess,
    sequence: u64,
}

struct Cached {
    batch: EditBatch,
    receipt: EditReceipt,
    bytes: usize,
}

/// Sole mutable owner of an edit document, trusted grants and bounded request history.
pub struct EditAuthority {
    document: Document,
    limits: EditLimits,
    clients: BTreeMap<EditorId, Client>,
    receipts: VecDeque<Cached>,
    receipt_bytes: usize,
    sequence: u64,
    poisoned: bool,
}

impl EditAuthority {
    /// Opens a World for edit-only use. Initial objects receive editor-zero IDs in tree order.
    /// Keep this ordered baseline identical when constructing replicas; future snapshot adapters
    /// must carry the stable identity association. The caller may subscribe before transferring ownership.
    pub fn new(world: World, limits: EditLimits) -> Result<Self, EditError> {
        validate_limits(limits)?;
        Ok(Self {
            document: Document::new(world, limits)?,
            limits,
            clients: BTreeMap::new(),
            receipts: VecDeque::new(),
            receipt_bytes: 0,
            sequence: 0,
            poisoned: false,
        })
    }

    /// Reads the original World without permitting out-of-band mutations.
    pub fn world(&self) -> &World {
        &self.document.world
    }

    /// Ends the edit session, retaining the original World and its subscriptions.
    pub fn into_world(self) -> World {
        self.document.world
    }

    /// Last accepted global sequence, starting at zero before any acceptance.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Looks up a live local handle; destroyed identities remain invalid forever.
    pub fn instance_of(&self, id: EditId) -> Option<InstanceId> {
        self.document.instance_of(id)
    }

    /// Maps a local handle to its stable document identity.
    pub fn id_of(&self, id: InstanceId) -> Option<EditId> {
        self.document.id_of(id)
    }

    /// Returns deterministic semantic state, including root/child order and stable references.
    /// Allocates in proportion to the document; this is a snapshot operation, not a frame update.
    pub fn snapshot(&self) -> Result<EditSnapshot, EditError> {
        self.document.snapshot()
    }

    /// Trusted host grant/revocation. Lifetime client slots/counters are never recycled by revocation.
    /// This is not a command an unauthenticated editor may invoke.
    pub fn set_access(&mut self, editor: EditorId, access: EditAccess) -> Result<(), EditError> {
        if self.poisoned {
            return Err(EditError::Poisoned);
        }
        if editor.0 == 0 {
            return Err(EditError::ReservedEditor);
        }
        if let Some(client) = self.clients.get_mut(&editor) {
            client.access = access;
            return Ok(());
        }
        if self.clients.len() >= self.limits.max_clients {
            return Err(EditError::ClientLimit);
        }
        self.clients.insert(
            editor,
            Client {
                access,
                sequence: 0,
            },
        );
        Ok(())
    }

    /// Trusted host assignment of whole-script write ownership, with no timer/lease claim.
    /// Newly created scripts start owned by their authenticated creator. Revocation never rewrites text.
    pub fn set_script_owner(
        &mut self,
        id: EditId,
        owner: Option<EditorId>,
    ) -> Result<(), EditError> {
        if self.poisoned {
            return Err(EditError::Poisoned);
        }
        if let Some(owner) = owner
            && !self
                .clients
                .get(&owner)
                .is_some_and(|client| client.access.can_edit())
        {
            return Err(EditError::PermissionDenied);
        }
        self.document.set_script_owner(id, owner)
    }

    /// Validates then commits a batch once. A rejected batch leaves the client's next sequence available
    /// for correction/retry. Accepted payload equality is exact (numeric bits included); cached repeats
    /// acknowledge only. Older evicted requests are rejected rather than reapplied.
    ///
    /// Ordinary validation/access failures are atomic rejections. [`EditError::ReplayFault`] is instead
    /// a terminal internal failure: callbacks/partial live writes may already have occurred, no accepted
    /// receipt is emitted, and all subsequent submissions fail closed until the document is reloaded.
    pub fn submit(&mut self, editor: EditorId, batch: &EditBatch) -> Result<Submission, EditError> {
        if self.poisoned {
            return Err(EditError::Poisoned);
        }
        if editor.0 == 0 {
            return Err(EditError::ReservedEditor);
        }
        let client = self
            .clients
            .get(&editor)
            .filter(|client| client.access.can_edit())
            .ok_or(EditError::PermissionDenied)?;
        let bytes = measure(batch, self.limits)?;
        if batch.client_sequence <= client.sequence {
            return match self.receipts.iter().find(|cached| {
                cached.receipt.editor == editor
                    && cached.receipt.client_sequence == batch.client_sequence
            }) {
                Some(cached) if cached.batch == *batch => Ok(Submission::Duplicate(cached.receipt)),
                Some(_) => Err(EditError::RequestReuse),
                None => Err(EditError::StaleRequest),
            };
        }
        let expected = client
            .sequence
            .checked_add(1)
            .ok_or(EditError::SequenceExhausted)?;
        if batch.client_sequence != expected {
            return Err(EditError::ClientSequence { expected });
        }
        let next = self
            .sequence
            .checked_add(1)
            .ok_or(EditError::SequenceExhausted)?;
        let prepared = self.document.stage(editor, batch, self.limits, true)?;
        self.commit_prepared(editor, batch, bytes, next, &prepared)
    }

    fn commit_prepared(
        &mut self,
        editor: EditorId,
        batch: &EditBatch,
        bytes: usize,
        next: u64,
        prepared: &Bindings,
    ) -> Result<Submission, EditError> {
        // Set first so even a callback panic caught by a caller cannot permit another partial replay.
        self.poisoned = true;
        if self
            .document
            .replay(editor, batch, self.limits, true, prepared)
            .is_err()
        {
            return Err(EditError::ReplayFault);
        }
        let receipt = EditReceipt {
            server_sequence: next,
            editor,
            client_sequence: batch.client_sequence,
        };
        while self.receipts.len() >= self.limits.max_receipts
            || self
                .receipt_bytes
                .checked_add(bytes)
                .is_none_or(|total| total > self.limits.max_receipt_bytes)
        {
            let Some(old) = self.receipts.pop_front() else {
                return Err(EditError::ReplayFault);
            };
            let Some(remaining) = self.receipt_bytes.checked_sub(old.bytes) else {
                return Err(EditError::ReplayFault);
            };
            self.receipt_bytes = remaining;
        }
        let Some(total) = self.receipt_bytes.checked_add(bytes) else {
            return Err(EditError::ReplayFault);
        };
        let Some(client) = self.clients.get_mut(&editor) else {
            return Err(EditError::ReplayFault);
        };
        // Clone normalizes retained Vec/String capacity to the bounded payload, not caller spare capacity.
        self.receipts.push_back(Cached {
            batch: batch.clone(),
            receipt,
            bytes,
        });
        self.receipt_bytes = total;
        client.sequence = batch.client_sequence;
        self.sequence = next;
        self.poisoned = false;
        Ok(Submission::Committed(CommittedBatch {
            receipt,
            batch: batch.clone(),
        }))
    }
}

/// Read-only document replica consuming a trusted authority's globally ordered committed records.
/// Local instance allocation can differ; every reference/order operation resolves stable IDs.
pub struct EditReplica {
    document: Document,
    limits: EditLimits,
    sequence: u64,
    clients: BTreeMap<EditorId, u64>,
    last: Option<CommittedBatch>,
    poisoned: bool,
}

impl EditReplica {
    /// Opens the same ordered initial semantic tree as the authority, independently of local slot history.
    pub fn new(world: World, limits: EditLimits) -> Result<Self, EditError> {
        validate_limits(limits)?;
        Ok(Self {
            document: Document::new(world, limits)?,
            limits,
            sequence: 0,
            clients: BTreeMap::new(),
            last: None,
            poisoned: false,
        })
    }

    /// Reads the replica World without unlogged local writes.
    pub fn world(&self) -> &World {
        &self.document.world
    }

    /// Last applied global sequence.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Deterministic semantic snapshot, allocating in proportion to the document.
    pub fn snapshot(&self) -> Result<EditSnapshot, EditError> {
        self.document.snapshot()
    }

    /// Resolves a stable live identity in this replica's local slot space.
    pub fn instance_of(&self, id: EditId) -> Option<InstanceId> {
        self.document.instance_of(id)
    }

    /// Applies the next trusted record. An exact repeat of the last record is a no-op;
    /// gaps/changed repeats reject without mutation. Source ownership/host access was already checked
    /// by the authority, not authenticated here. No optimistic pending-state behavior is implemented.
    pub fn apply(&mut self, record: &CommittedBatch) -> Result<(), EditError> {
        if self.poisoned {
            return Err(EditError::Poisoned);
        }
        let next = self.sequence.checked_add(1);
        if self.last.as_ref() == Some(record) {
            return Ok(());
        }
        let next = next.ok_or(EditError::SequenceExhausted)?;
        if record.receipt.server_sequence != next {
            return Err(EditError::ServerSequence { expected: next });
        }
        if record.receipt.editor.0 == 0 {
            return Err(EditError::ReservedEditor);
        }
        let expected = self
            .clients
            .get(&record.receipt.editor)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(EditError::SequenceExhausted)?;
        if record.batch.client_sequence != expected || record.receipt.client_sequence != expected {
            return Err(EditError::ClientSequence { expected });
        }
        if !self.clients.contains_key(&record.receipt.editor)
            && self.clients.len() >= self.limits.max_clients
        {
            return Err(EditError::ClientLimit);
        }
        measure(&record.batch, self.limits)?;
        let prepared =
            self.document
                .stage(record.receipt.editor, &record.batch, self.limits, false)?;
        self.poisoned = true;
        if self
            .document
            .replay(
                record.receipt.editor,
                &record.batch,
                self.limits,
                false,
                &prepared,
            )
            .is_err()
        {
            return Err(EditError::ReplayFault);
        }
        self.sequence = next;
        self.clients.insert(record.receipt.editor, expected);
        self.last = Some(record.clone());
        self.poisoned = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn callback_panic_and_unexpected_replay_error_are_terminal_not_atomic_rejections() {
        let mut world = World::new_empty();
        let part = world.create("Part", None).expect("part");
        world
            .property_changed(part, "Transparency")
            .connect(|_| panic!("controlled handler failure"));
        let mut authority = EditAuthority::new(world, EditLimits::default()).expect("authority");
        authority
            .set_access(EditorId(1), EditAccess::Edit)
            .expect("access");
        let id = authority.id_of(part).expect("id");
        let batch = EditBatch {
            client_sequence: 1,
            operations: vec![EditOp::Set {
                id,
                properties: vec![("Transparency".into(), EditValue::Number(0.5))],
            }],
        };
        assert!(catch_unwind(AssertUnwindSafe(|| authority.submit(EditorId(1), &batch))).is_err());
        assert_eq!(authority.sequence(), 0);
        assert_eq!(
            authority.submit(EditorId(1), &batch),
            Err(EditError::Poisoned)
        );

        let mut authority =
            EditAuthority::new(World::new_empty(), EditLimits::default()).expect("authority");
        authority
            .set_access(EditorId(1), EditAccess::Edit)
            .expect("access");
        let batch = EditBatch {
            client_sequence: 1,
            operations: vec![EditOp::Create {
                id: EditId::new(1, 1),
                class: "Part".into(),
                parent: None,
                after: None,
                properties: Vec::new(),
            }],
        };
        let prepared = authority
            .document
            .stage(EditorId(1), &batch, authority.limits, true)
            .expect("preflight");
        // Deliberately violate the private invariant between prepare/replay; public ownership cannot do this.
        authority
            .document
            .world
            .create("Folder", None)
            .expect("injected private discrepancy");
        let bytes = measure(&batch, authority.limits).expect("bytes");
        assert_eq!(
            authority.commit_prepared(EditorId(1), &batch, bytes, 1, &prepared),
            Err(EditError::ReplayFault)
        );
        assert_eq!(authority.sequence(), 0);
        assert_eq!(
            authority.submit(EditorId(1), &batch),
            Err(EditError::Poisoned)
        );
    }
}
