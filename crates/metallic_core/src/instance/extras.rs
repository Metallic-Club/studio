//! Attributes: named values attached to any instance, outside the class registry.
//!
//! They live in a boxed side record of the node ([`Extras`]), so an instance without attributes
//! pays one `None` pointer and the property paths (`get`, `set`) never look at them. Changes are
//! recorded in the ordinary change journal (the instance counts as changed) and in the
//! attribute log, which tells readers *which* attribute changed: scripts use it to fire
//! `AttributeChanged`.
//!
//! Replication (#305) will treat an attribute as a property-like value of its instance; the
//! registry there must list the allowed [`Value`] types the same way [`check_attribute`] does.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{InstanceId, World};
use crate::CoreError;
use crate::value::Value;

/// Longest attribute name, in bytes (Roblox: 100).
pub const MAX_ATTRIBUTE_NAME: usize = 100;
/// Longest string attribute value, in bytes.
pub const MAX_ATTRIBUTE_STRING: usize = 65_536;
/// Most attributes one instance can hold.
pub const MAX_ATTRIBUTES: usize = 1024;
/// Most attribute and tag changes a log keeps before the oldest half is dropped. Readers that
/// follow the log trim it ([`World::trim_attribute_log`]), so this only matters when changes
/// pile up unread (hundreds of thousands in one step).
const LOG_LIMIT: usize = 262_144;
/// A log with room for more entries than this is shrunk once it is mostly empty again.
const SHRINK_ABOVE: usize = 4096;
/// Most bytes of attribute data (names, values; tags too) one world holds, 16 MiB. Scripts
/// allocate this outside the VM's own memory cap, so it is budgeted here.
pub const MAX_EXTRAS_BYTES: usize = 16 * 1024 * 1024;

/// What a node holds besides class, parent, children and registry properties.
#[derive(Clone, Default)]
pub(super) struct Extras {
    pub(super) attributes: BTreeMap<String, Value>,
    pub(super) tags: BTreeSet<String>,
}

impl Extras {
    fn is_empty(&self) -> bool {
        self.attributes.is_empty() && self.tags.is_empty()
    }
}

/// A position in a world's attribute log; see [`World::attribute_changes_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeCursor {
    world: u64,
    seq: u64,
}

impl AttributeCursor {
    /// The cursor after `n` more changes were handled.
    pub fn after(self, n: usize) -> AttributeCursor {
        AttributeCursor {
            world: self.world,
            seq: self.seq + n as u64,
        }
    }

    /// How many changes were logged between this cursor and the later `newer` one.
    pub fn changes_until(self, newer: AttributeCursor) -> usize {
        newer.seq.saturating_sub(self.seq) as usize
    }
}

/// One attribute that changed (set, changed value, or removed). Read the current value from the
/// world: `None` there means it was removed (or the instance is gone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeChange {
    /// The instance.
    pub id: InstanceId,
    /// The attribute name.
    pub name: String,
}

/// Attribute changes since an [`AttributeCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeChanges {
    /// In order, one entry per accepted change.
    Changes(Vec<AttributeChange>),
    /// The log no longer reaches the cursor (or it is from another world): reread everything.
    Overflow,
}

/// A bounded log of changes with sequence numbers, read by cursors.
pub(super) struct ChangeLog<T> {
    entries: VecDeque<(u64, T)>,
    seq: u64,
    /// Entries with a sequence number up to this were dropped.
    floor: u64,
}

impl<T> Default for ChangeLog<T> {
    fn default() -> Self {
        ChangeLog {
            entries: VecDeque::new(),
            seq: 0,
            floor: 0,
        }
    }
}

impl<T: Clone> ChangeLog<T> {
    fn push(&mut self, change: T) {
        self.seq += 1;
        self.entries.push_back((self.seq, change));
        if self.entries.len() > LOG_LIMIT {
            let cut = self.entries.len() / 2;
            self.floor = self.entries[cut - 1].0;
            self.entries.drain(..cut);
        }
    }

    /// The entries after sequence number `seq`, or `None` when the log was cut past it.
    fn since(&self, seq: u64) -> Option<Vec<T>> {
        if seq < self.floor {
            return None;
        }
        let start = self.entries.partition_point(|(s, _)| *s <= seq);
        Some(
            self.entries
                .iter()
                .skip(start)
                .map(|(_, c)| c.clone())
                .collect(),
        )
    }

    /// At most `max` of the entries after sequence number `seq`, or `None` when the log was cut
    /// past it. Entries are numbered consecutively, so a reader that handled `n` of them is
    /// at `seq + n`.
    fn page_since(&self, seq: u64, max: usize) -> Option<Vec<T>> {
        if seq < self.floor {
            return None;
        }
        let start = self.entries.partition_point(|(s, _)| *s <= seq);
        Some(
            self.entries
                .iter()
                .skip(start)
                .take(max)
                .map(|(_, c)| c.clone())
                .collect(),
        )
    }

    /// Drops the entries up to and including sequence number `seq` (every reader is past them),
    /// and gives back the memory of a burst: a log that is mostly empty again keeps room for
    /// about twice what it holds, not for the biggest burst it ever saw.
    fn trim_to(&mut self, seq: u64) {
        let cut = self.entries.partition_point(|(s, _)| *s <= seq);
        if cut > 0 {
            self.floor = self.entries[cut - 1].0;
            self.entries.drain(..cut);
        }
        let capacity = self.entries.capacity();
        if capacity > SHRINK_ABOVE && self.entries.len() * 4 < capacity {
            self.entries
                .shrink_to((self.entries.len() * 2).max(SHRINK_ABOVE / 4));
        }
    }

    fn capacity(&self) -> usize {
        self.entries.capacity()
    }
}

pub(super) type AttributeLog = ChangeLog<AttributeChange>;
pub(super) type TagLog = ChangeLog<TagChange>;

/// Whether `name` is a valid attribute name: 1 to [`MAX_ATTRIBUTE_NAME`] bytes of ASCII letters,
/// digits and underscores, and not starting with `RBX` (reserved, any case).
pub fn check_attribute_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("attribute names cannot be empty".into());
    }
    if name.len() > MAX_ATTRIBUTE_NAME {
        return Err(format!(
            "attribute names are at most {MAX_ATTRIBUTE_NAME} characters"
        ));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("attribute names can only contain letters, digits and underscores".into());
    }
    if name.len() >= 3 && name[..3].eq_ignore_ascii_case("RBX") {
        return Err("attribute names cannot start with RBX".into());
    }
    Ok(())
}

/// Checks that `value` can be stored as an attribute: a bool, number, string, Vector3, Color3,
/// UDim2 or CFrame, with finite numbers and a string of at most [`MAX_ATTRIBUTE_STRING`] bytes.
/// `Nil` is accepted (it removes the attribute).
pub fn check_attribute_value(value: &Value) -> Result<(), String> {
    match value {
        Value::Ref(_) => {
            return Err("attributes cannot hold instance references".into());
        }
        Value::String(s) if s.len() > MAX_ATTRIBUTE_STRING => {
            return Err(format!(
                "string attributes are at most {MAX_ATTRIBUTE_STRING} bytes"
            ));
        }
        _ => {}
    }
    if !value.has_finite_components() {
        return Err("all numeric components must be finite".into());
    }
    Ok(())
}

/// What an attribute costs against [`MAX_EXTRAS_BYTES`]: its name, its value, a fixed overhead.
fn entry_cost(name: &str, value: &Value) -> usize {
    let value = match value {
        Value::String(s) => s.len(),
        Value::CFrame(_) => 96,
        Value::Vector3(_) | Value::Color3(_) => 24,
        Value::UDim2(_) => 32,
        _ => 8,
    };
    name.len() + value + 48
}

/// What the attributes of one node cost against [`MAX_EXTRAS_BYTES`].
pub(super) fn extras_cost(extras: &Extras) -> usize {
    extras
        .attributes
        .iter()
        .map(|(name, value)| entry_cost(name, value))
        .sum::<usize>()
        + extras.tags.iter().map(|t| tag_cost(t)).sum::<usize>()
}

/// What a tag costs against [`MAX_EXTRAS_BYTES`].
fn tag_cost(tag: &str) -> usize {
    tag.len() + 32
}

fn invalid(name: &str, reason: String) -> CoreError {
    CoreError::InvalidValue {
        property: name.to_string(),
        reason,
    }
}

impl World {
    /// Sets attribute `name` of `id` to `value`; `Value::Nil` removes it. Setting the value it
    /// already has (or removing one that is not there) changes nothing and records nothing.
    ///
    /// Errors: [`CoreError::InvalidInstance`]; [`CoreError::InvalidValue`] for a bad name (see
    /// [`check_attribute_name`]), a value type attributes cannot hold, a non-finite number, an
    /// oversized string, or an instance that already holds [`MAX_ATTRIBUTES`] attributes.
    pub fn set_attribute(
        &mut self,
        id: InstanceId,
        name: &str,
        value: Value,
    ) -> Result<(), CoreError> {
        if self.node(id).is_none() {
            return Err(CoreError::InvalidInstance);
        }
        check_attribute_name(name).map_err(|e| invalid(name, e))?;
        check_attribute_value(&value).map_err(|e| invalid(name, e))?;
        let removing = matches!(value, Value::Nil);
        let old_cost = self.attribute(id, name).map_or(0, |v| entry_cost(name, v));
        let new_cost = if removing {
            0
        } else {
            entry_cost(name, &value)
        };
        if !removing && self.extras_bytes - old_cost + new_cost > MAX_EXTRAS_BYTES {
            return Err(invalid(
                name,
                format!(
                    "the attribute data budget of a world is {} MiB",
                    MAX_EXTRAS_BYTES >> 20
                ),
            ));
        }
        let node = self.node_mut(id).ok_or(CoreError::InvalidInstance)?;
        if removing {
            let Some(extras) = node.extras.as_mut() else {
                return Ok(());
            };
            if extras.attributes.remove(name).is_none() {
                return Ok(());
            }
            if extras.is_empty() {
                node.extras = None;
            }
        } else {
            let extras = node.extras.get_or_insert_with(Default::default);
            match extras.attributes.get(name) {
                Some(old) if *old == value => return Ok(()),
                None if extras.attributes.len() >= MAX_ATTRIBUTES => {
                    if extras.is_empty() {
                        node.extras = None;
                    }
                    return Err(invalid(
                        name,
                        format!("an instance holds at most {MAX_ATTRIBUTES} attributes"),
                    ));
                }
                _ => {}
            }
            extras.attributes.insert(name.to_string(), value);
        }
        self.extras_bytes = self.extras_bytes - old_cost + new_cost;
        // Attributes and tags are coarse-only in the journal for now: `changes_since` reports
        // the id, `changes_since_detailed` reports nothing (use the attribute and tag logs).
        self.record(id, None);
        self.attribute_log.push(AttributeChange {
            id,
            name: name.to_string(),
        });
        Ok(())
    }

    /// The value of attribute `name`, or `None` when it is not set (or `id` is not alive).
    pub fn attribute(&self, id: InstanceId, name: &str) -> Option<&Value> {
        self.node(id)?.extras.as_ref()?.attributes.get(name)
    }

    /// All attributes of `id`, sorted by name; empty for an instance without any.
    pub fn attributes(&self, id: InstanceId) -> impl Iterator<Item = (&str, &Value)> {
        self.node(id)
            .and_then(|n| n.extras.as_ref())
            .into_iter()
            .flat_map(|e| e.attributes.iter().map(|(k, v)| (k.as_str(), v)))
    }

    /// Drops the attribute log entries every reader has already read: `readers` are the cursors
    /// of all of them (cursors of other worlds are ignored); with none, nothing is dropped. The
    /// script host calls this after it delivered a step, so the log only holds unread changes.
    pub fn trim_attribute_log(&mut self, readers: &[AttributeCursor]) {
        let uid = self.uid;
        let oldest = readers
            .iter()
            .filter(|c| c.world == uid)
            .map(|c| c.seq)
            .min();
        if let Some(seq) = oldest {
            self.attribute_log.trim_to(seq);
        }
    }

    /// How many bytes of attribute data the world holds, against [`MAX_EXTRAS_BYTES`].
    pub fn extras_bytes(&self) -> usize {
        self.extras_bytes
    }

    /// What the attributes of `id` and its descendants cost.
    pub(super) fn subtree_extras_cost(&self, id: InstanceId) -> usize {
        let mut total = 0;
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let Some(node) = self.node(cur) else { continue };
            if let Some(extras) = &node.extras {
                total += extras_cost(extras);
            }
            stack.extend(node.children.iter().copied());
        }
        total
    }

    /// At most `max` attribute changes after `cursor`, oldest first, for a reader that handles
    /// them a few at a time (it keeps the cursor at the first change it did not handle, see
    /// [`AttributeCursor::after`]). `Overflow` as in [`World::attribute_changes_since`].
    pub fn attribute_changes_page(&self, cursor: AttributeCursor, max: usize) -> AttributeChanges {
        if cursor.world != self.uid {
            return AttributeChanges::Overflow;
        }
        match self.attribute_log.page_since(cursor.seq, max) {
            Some(changes) => AttributeChanges::Changes(changes),
            None => AttributeChanges::Overflow,
        }
    }

    /// Drops every attribute and tag log entry and gives their memory back. For a world that
    /// nothing reads the logs of (the editor's edit world): its readers do not exist, so
    /// nothing may wait for them.
    pub fn clear_change_logs(&mut self) {
        let attributes = self.attribute_cursor();
        let tags = self.tag_cursor();
        self.trim_attribute_log(&[attributes]);
        self.trim_tag_log(&[tags]);
    }

    /// Entries of room the attribute and tag logs hold, for diagnostics and tests.
    pub fn change_log_capacity(&self) -> usize {
        self.attribute_log.capacity() + self.tag_log.capacity()
    }

    /// The current end of the attribute log.
    pub fn attribute_cursor(&self) -> AttributeCursor {
        AttributeCursor {
            world: self.uid,
            seq: self.attribute_log.seq,
        }
    }

    /// Attribute changes since `cursor`, plus a cursor to continue from. Needs only `&World`, so
    /// several readers (the server VM, the client VM, replication) each keep their own cursor.
    /// The log is bounded: when it no longer reaches `cursor`, or `cursor` came from another
    /// world, the result is [`AttributeChanges::Overflow`].
    pub fn attribute_changes_since(
        &self,
        cursor: AttributeCursor,
    ) -> (AttributeChanges, AttributeCursor) {
        let now = self.attribute_cursor();
        if cursor.world != self.uid {
            return (AttributeChanges::Overflow, now);
        }
        match self.attribute_log.since(cursor.seq) {
            Some(changes) => (AttributeChanges::Changes(changes), now),
            None => (AttributeChanges::Overflow, now),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tags

/// Longest tag, in bytes.
pub const MAX_TAG: usize = 255;
/// Most tags one instance can hold.
pub const MAX_TAGS: usize = 256;

/// Whether `tag` can be used: 1 to [`MAX_TAG`] bytes, no control characters.
pub fn check_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err("tags cannot be empty".into());
    }
    if tag.len() > MAX_TAG {
        return Err(format!("tags are at most {MAX_TAG} bytes"));
    }
    if tag.chars().any(char::is_control) {
        return Err("tags cannot contain control characters".into());
    }
    Ok(())
}

/// A position in a world's tag log; see [`World::tag_changes_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagCursor {
    world: u64,
    seq: u64,
}

impl TagCursor {
    /// The cursor after `n` more changes were handled.
    pub fn after(self, n: usize) -> TagCursor {
        TagCursor {
            world: self.world,
            seq: self.seq + n as u64,
        }
    }

    /// How many changes were logged between this cursor and the later `newer` one.
    pub fn changes_until(self, newer: TagCursor) -> usize {
        newer.seq.saturating_sub(self.seq) as usize
    }
}

/// One tag that became (or stopped being) active on an instance in the game tree.
///
/// "Active" is Roblox's rule: the instance carries the tag and descends from a service (it is
/// not detached). So a notice is logged when a tag is added or removed on an instance in the
/// tree, when a tagged subtree enters the tree (reparented under a service) or leaves it
/// (reparented to nothing or destroyed). Read [`World::has_tag`] for the current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagChange {
    /// The instance (it may be destroyed by now).
    pub id: InstanceId,
    /// The tag.
    pub tag: String,
    /// True when the tag became active, false when it stopped.
    pub added: bool,
}

/// Tag changes since a [`TagCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagChanges {
    /// In order.
    Changes(Vec<TagChange>),
    /// The log no longer reaches the cursor (or it is from another world): reread everything.
    Overflow,
}

impl World {
    /// Tags `id` with `tag`. Tagging twice is fine and records nothing.
    ///
    /// Errors: [`CoreError::InvalidInstance`]; [`CoreError::InvalidValue`] for a bad tag (see
    /// [`check_tag`]) or an instance that already holds [`MAX_TAGS`] tags.
    pub fn add_tag(&mut self, id: InstanceId, tag: &str) -> Result<(), CoreError> {
        if self.node(id).is_none() {
            return Err(CoreError::InvalidInstance);
        }
        check_tag(tag).map_err(|e| invalid(tag, e))?;
        if self.has_tag(id, tag) {
            return Ok(());
        }
        let cost = tag_cost(tag);
        if self.extras_bytes + cost > MAX_EXTRAS_BYTES {
            return Err(invalid(
                tag,
                format!(
                    "the attribute and tag data budget of a world is {} MiB",
                    MAX_EXTRAS_BYTES >> 20
                ),
            ));
        }
        let node = self.node_mut(id).ok_or(CoreError::InvalidInstance)?;
        let extras = node.extras.get_or_insert_with(Default::default);
        if extras.tags.len() >= MAX_TAGS {
            if extras.is_empty() {
                node.extras = None;
            }
            return Err(invalid(
                tag,
                format!("an instance holds at most {MAX_TAGS} tags"),
            ));
        }
        extras.tags.insert(tag.to_string());
        self.extras_bytes += cost;
        self.tag_index
            .entry(tag.to_string())
            .or_default()
            .insert(id);
        self.record(id, None); // coarse-only, like attributes
        if self.is_in_tree(id) {
            self.tag_log.push(TagChange {
                id,
                tag: tag.to_string(),
                added: true,
            });
        }
        Ok(())
    }

    /// Removes `tag` from `id`. Removing a tag that is not there is fine.
    ///
    /// Errors: [`CoreError::InvalidInstance`].
    pub fn remove_tag(&mut self, id: InstanceId, tag: &str) -> Result<(), CoreError> {
        let node = self.node_mut(id).ok_or(CoreError::InvalidInstance)?;
        let Some(extras) = node.extras.as_mut() else {
            return Ok(());
        };
        if !extras.tags.remove(tag) {
            return Ok(());
        }
        if extras.is_empty() {
            node.extras = None;
        }
        self.extras_bytes -= tag_cost(tag);
        if let Some(set) = self.tag_index.get_mut(tag) {
            set.remove(&id);
            if set.is_empty() {
                self.tag_index.remove(tag);
            }
        }
        self.record(id, None); // coarse-only, like attributes
        if self.is_in_tree(id) {
            self.tag_log.push(TagChange {
                id,
                tag: tag.to_string(),
                added: false,
            });
        }
        Ok(())
    }

    /// Whether `id` carries `tag`.
    pub fn has_tag(&self, id: InstanceId, tag: &str) -> bool {
        self.node(id)
            .and_then(|n| n.extras.as_ref())
            .is_some_and(|e| e.tags.contains(tag))
    }

    /// The tags of `id`, sorted; empty for an instance without any.
    pub fn tags(&self, id: InstanceId) -> impl Iterator<Item = &str> {
        self.node(id)
            .and_then(|n| n.extras.as_ref())
            .into_iter()
            .flat_map(|e| e.tags.iter().map(String::as_str))
    }

    /// The instances in the game tree that carry `tag`, in id order. Looked up in an index, not
    /// found by scanning the tree: the cost follows the number of instances with the tag.
    pub fn tagged(&self, tag: &str) -> Vec<InstanceId> {
        self.tag_index
            .get(tag)
            .map(|set| {
                set.iter()
                    .copied()
                    .filter(|&id| self.is_in_tree(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every tag some instance carries, sorted (detached instances included).
    pub fn all_tags(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self.tag_index.keys().map(String::as_str).collect();
        all.sort_unstable();
        all
    }

    /// True when `id` descends from a service (a built-in root), that is, it is part of the game
    /// rather than a detached tree (a fresh clone, or an instance with no parent).
    pub fn is_in_tree(&self, id: InstanceId) -> bool {
        let mut cur = id;
        // The depth is bounded by the number of slots; the loop cannot cycle (reparenting
        // refuses cycles), the bound only guards against a corrupted tree.
        for _ in 0..=self.slots.len() {
            match self.node(cur) {
                None => return false,
                Some(n) => match n.parent {
                    Some(p) => cur = p,
                    None => return n.builtin,
                },
            }
        }
        false
    }

    /// Drops the tag log entries every reader has already read; see
    /// [`World::trim_attribute_log`].
    pub fn trim_tag_log(&mut self, readers: &[TagCursor]) {
        let uid = self.uid;
        let oldest = readers
            .iter()
            .filter(|c| c.world == uid)
            .map(|c| c.seq)
            .min();
        if let Some(seq) = oldest {
            self.tag_log.trim_to(seq);
        }
    }

    /// The current end of the tag log.
    pub fn tag_cursor(&self) -> TagCursor {
        TagCursor {
            world: self.uid,
            seq: self.tag_log.seq,
        }
    }

    /// At most `max` tag changes after `cursor`, oldest first; the paged counterpart of
    /// [`World::tag_changes_since`] (see [`World::attribute_changes_page`]).
    pub fn tag_changes_page(&self, cursor: TagCursor, max: usize) -> TagChanges {
        if cursor.world != self.uid {
            return TagChanges::Overflow;
        }
        match self.tag_log.page_since(cursor.seq, max) {
            Some(changes) => TagChanges::Changes(changes),
            None => TagChanges::Overflow,
        }
    }

    /// Tag changes since `cursor` plus a cursor to continue from; like
    /// [`World::attribute_changes_since`], every reader keeps its own cursor and the log is
    /// bounded ([`TagChanges::Overflow`] when it no longer reaches back).
    pub fn tag_changes_since(&self, cursor: TagCursor) -> (TagChanges, TagCursor) {
        let now = self.tag_cursor();
        if cursor.world != self.uid {
            return (TagChanges::Overflow, now);
        }
        match self.tag_log.since(cursor.seq) {
            Some(changes) => (TagChanges::Changes(changes), now),
            None => (TagChanges::Overflow, now),
        }
    }

    /// Logs every tag of the tagged instances in the subtree of `id` as active or not. Called
    /// when the subtree enters or leaves the game tree.
    pub(super) fn log_subtree_tags(&mut self, id: InstanceId, added: bool) {
        let mut stack = vec![id];
        let mut out = Vec::new();
        while let Some(cur) = stack.pop() {
            let Some(node) = self.node(cur) else { continue };
            if let Some(extras) = &node.extras {
                for tag in &extras.tags {
                    out.push(TagChange {
                        id: cur,
                        tag: tag.clone(),
                        added,
                    });
                }
            }
            stack.extend(node.children.iter().rev().copied());
        }
        for change in out {
            self.tag_log.push(change);
        }
    }

    /// Drops the tags of the subtree of `id` from the index (it is being destroyed).
    pub(super) fn forget_subtree_tags(&mut self, id: InstanceId) {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let Some(node) = self.node(cur) else { continue };
            let tags: Vec<String> = node
                .extras
                .as_ref()
                .map(|e| e.tags.iter().cloned().collect())
                .unwrap_or_default();
            stack.extend(node.children.iter().copied());
            for tag in tags {
                if let Some(set) = self.tag_index.get_mut(&tag) {
                    set.remove(&cur);
                    if set.is_empty() {
                        self.tag_index.remove(&tag);
                    }
                }
            }
        }
    }

    /// Adds the tags of a freshly cloned instance to the index.
    pub(super) fn index_tags_of(&mut self, id: InstanceId) {
        let tags: Vec<String> = self.tags(id).map(str::to_string).collect();
        for tag in tags {
            self.tag_index.entry(tag).or_default().insert(id);
        }
    }
}
