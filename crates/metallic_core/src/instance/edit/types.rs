//! Values and limits for the edit-only command boundary.

use crate::{Color3, CoreError, UDim2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An editor identity supplied by the trusted caller, not authenticated by this module.
/// Zero is reserved for objects present when the document is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EditorId(
    /// Host-provided numeric identity, with zero reserved for initial objects.
    pub u64,
);

/// A document identity independent of a process's local instance slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EditId {
    /// Creator namespace; zero identifies the initial document.
    pub editor: EditorId,
    /// Positive, strictly increasing creation counter in that namespace.
    pub counter: u64,
}

impl EditId {
    /// Constructs an identity; the authority validates counters and creator ownership.
    pub const fn new(editor: u64, counter: u64) -> Self {
        Self {
            editor: EditorId(editor),
            counter,
        }
    }
}

/// A property value whose references always use stable document identities.
/// Numeric request equality compares bits, including the sign of zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditValue {
    /// No value.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A number; World validation rejects non-finite values.
    Number(f64),
    /// UTF-8 text.
    String(String),
    /// A three-dimensional vector.
    Vector3(Vec3),
    /// An RGB colour.
    Color3(Color3),
    /// A GUI size or position.
    UDim2(UDim2),
    /// A live stable reference, or no target. Raw instance handles are not accepted.
    Ref(Option<EditId>),
}

impl PartialEq for EditValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Vector3(a), Self::Vector3(b)) => {
                [a.x.to_bits(), a.y.to_bits(), a.z.to_bits()]
                    == [b.x.to_bits(), b.y.to_bits(), b.z.to_bits()]
            }
            (Self::Color3(a), Self::Color3(b)) => {
                [a.r.to_bits(), a.g.to_bits(), a.b.to_bits()]
                    == [b.r.to_bits(), b.g.to_bits(), b.b.to_bits()]
            }
            (Self::UDim2(a), Self::UDim2(b)) => {
                [
                    a.x_scale.to_bits(),
                    a.x_offset.to_bits(),
                    a.y_scale.to_bits(),
                    a.y_offset.to_bits(),
                ] == [
                    b.x_scale.to_bits(),
                    b.x_offset.to_bits(),
                    b.y_scale.to_bits(),
                    b.y_offset.to_bits(),
                ]
            }
            (Self::Ref(a), Self::Ref(b)) => a == b,
            _ => false,
        }
    }
}

/// One typed edit. Operations in a batch execute in their listed order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EditOp {
    /// Creates an object with properties. A new script's write owner is its authenticated creator.
    Create {
        /// Fresh identity belonging to the submitting editor.
        id: EditId,
        /// Registry class name, validated by World.
        class: String,
        /// Destination parent, or a root.
        parent: Option<EditId>,
        /// Existing destination sibling to follow; None appends.
        after: Option<EditId>,
        /// Ordered property assignments; Source shares script ownership checks.
        properties: Vec<(String, EditValue)>,
    },
    /// Sets ordered properties on an existing object.
    Set {
        /// Target identity.
        id: EditId,
        /// Ordered assignments.
        properties: Vec<(String, EditValue)>,
    },
    /// Reparents/reorders an existing object. Invalid anchors reject rather than append.
    Move {
        /// Target identity.
        id: EditId,
        /// Destination parent, or a root.
        parent: Option<EditId>,
        /// Existing destination sibling to follow; None appends.
        after: Option<EditId>,
    },
    /// Destroys an object and descendants. Stable creation counters are never reclaimed.
    Destroy {
        /// Subtree root.
        id: EditId,
    },
    /// Whole-text replacement under the same write-ownership check used by Source properties.
    ScriptText {
        /// Script target.
        id: EditId,
        /// Complete replacement text; this is not a text CRDT.
        text: String,
    },
}

/// A single atomic validation unit and future undo step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditBatch {
    /// Starts at one and increases only after acceptance. Rejected input may be corrected and retried.
    pub client_sequence: u64,
    /// Ordered operations; references to newly created objects must follow their creation.
    pub operations: Vec<EditOp>,
}

/// Acknowledgement of an accepted batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditReceipt {
    /// Global sequence, starting at one, assigned once per accepted batch.
    pub server_sequence: u64,
    /// Authenticated editor supplied by the host.
    pub editor: EditorId,
    /// The editor's accepted client sequence.
    pub client_sequence: u64,
}

/// An accepted ordered record, ready for a caller to broadcast or persist.
/// This module does neither transport nor persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedBatch {
    /// Assigned acknowledgement.
    pub receipt: EditReceipt,
    /// Accepted operations.
    pub batch: EditBatch,
}

/// Separates a new broadcast from a duplicate request's acknowledgement.
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    /// Apply/broadcast this record once.
    Committed(CommittedBatch),
    /// Acknowledge only; do not broadcast or reapply an old record.
    Duplicate(EditReceipt),
}

impl Submission {
    /// The original acknowledgement, including on duplicate delivery.
    pub fn receipt(&self) -> EditReceipt {
        match self {
            Self::Committed(record) => record.receipt,
            Self::Duplicate(receipt) => *receipt,
        }
    }
}

/// Trusted caller-provided access. Membership/authentication belongs to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAccess {
    /// Authoring permission; membership management remains outside this kernel.
    Owner,
    /// Authoring permission.
    Edit,
    /// Read/test permission only.
    Play,
    /// No authoring permission.
    None,
}

impl EditAccess {
    pub(super) fn can_edit(self) -> bool {
        matches!(self, Self::Owner | Self::Edit)
    }
}

/// Retained metadata and input limits. These do not cap the caller's existing World allocation.
#[derive(Debug, Clone, Copy)]
pub struct EditLimits {
    /// Maximum lifetime editor identities; removing access does not forget counters.
    pub max_clients: usize,
    /// Maximum operations per batch.
    pub max_operations: usize,
    /// Maximum batch bytes, including owned operation/property metadata and text.
    pub max_batch_bytes: usize,
    /// Maximum UTF-8 bytes for any Source assignment.
    pub max_script_bytes: usize,
    /// Maximum cached exact request/receipt pairs across all clients.
    pub max_receipts: usize,
    /// Maximum accounted cached request bytes across all clients.
    pub max_receipt_bytes: usize,
    /// Maximum live stable bindings, including initial services.
    pub max_instances: usize,
}

impl Default for EditLimits {
    fn default() -> Self {
        Self {
            max_clients: 32,
            max_operations: 256,
            max_batch_bytes: 256 * 1024,
            max_script_bytes: 128 * 1024,
            max_receipts: 64,
            max_receipt_bytes: 8 * 1024 * 1024,
            max_instances: 100_000,
        }
    }
}

/// A deterministic semantic snapshot for comparisons/snapshot adapters; not a wire protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditNode {
    /// Stable identity.
    pub id: EditId,
    /// Registry class.
    pub class: String,
    /// Stable parent.
    pub parent: Option<EditId>,
    /// Ordered stable children.
    pub children: Vec<EditId>,
    /// Effective properties, including defaults, with stable references.
    pub properties: BTreeMap<String, EditValue>,
}

/// Stable semantic state, including root and child ordering, for snapshot adapters/comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditSnapshot {
    /// Roots in document order.
    pub roots: Vec<EditId>,
    /// Live nodes sorted by stable identity.
    pub nodes: Vec<EditNode>,
}

/// Rejections leave live state unchanged, except the explicitly terminal replay fault.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    /// Invalid host limits.
    #[error("invalid edit limits")]
    InvalidLimits,
    /// The zero editor is reserved for initial objects.
    #[error("editor zero is reserved")]
    ReservedEditor,
    /// The lifetime client bound was reached.
    #[error("editor identity limit reached")]
    ClientLimit,
    /// No Edit permission was supplied by the trusted host.
    #[error("editor has no authoring permission")]
    PermissionDenied,
    /// A named input/retention bound was exceeded.
    #[error("edit limit exceeded: {0}")]
    Limit(&'static str),
    /// The client must submit its next accepted sequence.
    #[error("expected client sequence {expected}")]
    ClientSequence {
        /// Next sequence expected.
        expected: u64,
    },
    /// A previously accepted identifier has a different payload.
    #[error("accepted request identity reused with different content")]
    RequestReuse,
    /// An old receipt was evicted; never reapply it. The caller must resynchronize.
    #[error("request is older than retained receipt history")]
    StaleRequest,
    /// A global or client counter cannot advance without wrapping.
    #[error("edit sequence exhausted")]
    SequenceExhausted,
    /// Target or referenced identity is absent/deleted.
    #[error("unknown or deleted edit identity {0:?}")]
    InvalidObject(EditId),
    /// A Create tried to use someone else's namespace or an old creation counter.
    #[error("creation identity is not fresh for its authenticated creator")]
    CreationReuse,
    /// Missing, self, or wrong-parent anchor. The caller can rebase/retry.
    #[error("invalid destination sibling anchor {0:?}")]
    InvalidAnchor(EditId),
    /// Built-in services cannot be created, renamed, moved or destroyed through this boundary.
    #[error("built-in service structure is protected")]
    ProtectedBuiltin,
    /// A text/ownership operation targeted an object without Source.
    #[error("target is not a script")]
    NotScript,
    /// The host's script write owner is another editor or absent.
    #[error("editor does not own this script's write permission")]
    ScriptNotOwned,
    /// World schema validation failed at an operation, before live replay.
    #[error("operation {operation} rejected: {source}")]
    Core {
        /// Zero-based operation index.
        operation: usize,
        /// Existing World error; its validation is not duplicated here.
        source: CoreError,
    },
    /// A replica needs the next global record, not a gap or altered repeat.
    #[error("expected server sequence {expected}")]
    ServerSequence {
        /// Next sequence expected.
        expected: u64,
    },
    /// Internal replay invariant failure. Partial accepted callbacks may have run; reload is required.
    #[error("validated replay invariant failed; authority is poisoned and must be reloaded")]
    ReplayFault,
    /// A previous invariant fault made subsequent submission unsafe.
    #[error("authority is poisoned; reload is required")]
    Poisoned,
    /// Stable bindings and the owned World disagree; this is not an ordinary input rejection.
    #[error("edit document bindings are inconsistent")]
    InvalidDocument,
}
