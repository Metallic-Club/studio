//! World-backed staging and stable bindings. No second instance/property model is maintained.

use super::{
    EditBatch, EditError, EditId, EditLimits, EditNode, EditOp, EditSnapshot, EditValue, EditorId,
};
use crate::{InstanceId, Value, ValueType, World, classes};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Bindings {
    ids: BTreeMap<EditId, InstanceId>,
    reverse: BTreeMap<InstanceId, EditId>,
    creators: BTreeMap<EditorId, u64>,
    script_owners: BTreeMap<EditId, EditorId>,
}

pub(super) struct Document {
    pub(super) world: World,
    bindings: Bindings,
}

impl Document {
    pub(super) fn new(world: World, limits: EditLimits) -> Result<Self, EditError> {
        let mut bindings = Bindings {
            ids: BTreeMap::new(),
            reverse: BTreeMap::new(),
            creators: BTreeMap::new(),
            script_owners: BTreeMap::new(),
        };
        let mut stack: Vec<_> = world.roots().iter().rev().copied().collect();
        let mut seen = BTreeSet::new();
        let mut counter = 0u64;
        while let Some(local) = stack.pop() {
            if !world.is_alive(local) || !seen.insert(local) {
                return Err(EditError::InvalidDocument);
            }
            if bindings.ids.len() >= limits.max_instances {
                return Err(EditError::Limit("live instances"));
            }
            counter = counter.checked_add(1).ok_or(EditError::SequenceExhausted)?;
            let stable = EditId::new(0, counter);
            bindings.ids.insert(stable, local);
            bindings.reverse.insert(local, stable);
            stack.extend(world.children(local).iter().rev().copied());
        }
        bindings.creators.insert(EditorId(0), counter);
        Ok(Self { world, bindings })
    }

    pub(super) fn instance_of(&self, id: EditId) -> Option<InstanceId> {
        self.bindings
            .ids
            .get(&id)
            .copied()
            .filter(|local| self.world.is_alive(*local))
    }

    pub(super) fn id_of(&self, id: InstanceId) -> Option<EditId> {
        self.bindings.reverse.get(&id).copied()
    }

    fn resolve(&self, id: EditId) -> Result<InstanceId, EditError> {
        let local = self
            .bindings
            .ids
            .get(&id)
            .copied()
            .ok_or(EditError::InvalidObject(id))?;
        if !self.world.is_alive(local) {
            return Err(EditError::InvalidDocument);
        }
        Ok(local)
    }

    fn parent(&self, id: Option<EditId>) -> Result<Option<InstanceId>, EditError> {
        id.map(|id| self.resolve(id)).transpose()
    }

    fn is_script(&self, local: InstanceId) -> bool {
        self.world
            .class_name(local)
            .and_then(|class| classes::find_property(class, "Source"))
            .is_some_and(|property| property.ty == ValueType::String)
    }

    pub(super) fn set_script_owner(
        &mut self,
        id: EditId,
        owner: Option<EditorId>,
    ) -> Result<(), EditError> {
        let local = self.resolve(id)?;
        if !self.is_script(local) {
            return Err(EditError::NotScript);
        }
        match owner {
            Some(owner) => {
                self.bindings.script_owners.insert(id, owner);
            }
            None => {
                self.bindings.script_owners.remove(&id);
            }
        }
        Ok(())
    }

    pub(super) fn stage(
        &self,
        editor: EditorId,
        batch: &EditBatch,
        limits: EditLimits,
        check_owner: bool,
    ) -> Result<Bindings, EditError> {
        // deep_clone copies slot/generation/free-list state and installs no original handlers.
        let mut candidate = Self {
            world: self.world.deep_clone(),
            bindings: self.bindings.clone(),
        };
        candidate.apply(editor, batch, limits, check_owner)?;
        Ok(candidate.bindings)
    }

    pub(super) fn replay(
        &mut self,
        editor: EditorId,
        batch: &EditBatch,
        limits: EditLimits,
        check_owner: bool,
        expected: &Bindings,
    ) -> Result<(), EditError> {
        self.apply(editor, batch, limits, check_owner)?;
        if &self.bindings != expected {
            return Err(EditError::InvalidDocument);
        }
        Ok(())
    }

    fn validate_anchor(
        &self,
        target: EditId,
        parent: Option<InstanceId>,
        after: Option<EditId>,
    ) -> Result<Option<InstanceId>, EditError> {
        let Some(anchor) = after else { return Ok(None) };
        if anchor == target {
            return Err(EditError::InvalidAnchor(anchor));
        }
        let local = self
            .resolve(anchor)
            .map_err(|_| EditError::InvalidAnchor(anchor))?;
        let siblings = match parent {
            Some(parent) => self.world.children(parent),
            None => self.world.roots(),
        };
        if !siblings.contains(&local) {
            return Err(EditError::InvalidAnchor(anchor));
        }
        Ok(Some(local))
    }

    fn order_after(
        &mut self,
        id: InstanceId,
        parent: Option<InstanceId>,
        after: Option<InstanceId>,
        operation: usize,
    ) -> Result<(), EditError> {
        let index = match after {
            Some(anchor) => {
                let siblings = match parent {
                    Some(parent) => self.world.children(parent),
                    None => self.world.roots(),
                };
                // Core's target index is the final position, after removing this node.
                siblings
                    .iter()
                    .copied()
                    .filter(|&sibling| sibling != id)
                    .position(|sibling| sibling == anchor)
                    .ok_or(EditError::InvalidDocument)?
                    + 1
            }
            None => usize::MAX,
        };
        self.world
            .set_parent_at(id, parent, index)
            .map_err(|source| EditError::Core { operation, source })
    }

    fn subtree(&self, root: InstanceId) -> Result<Vec<InstanceId>, EditError> {
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if self.world.is_builtin(id) {
                return Err(EditError::ProtectedBuiltin);
            }
            result.push(id);
            stack.extend_from_slice(self.world.children(id));
        }
        Ok(result)
    }

    fn value(&self, value: &EditValue) -> Result<Value, EditError> {
        Ok(match value {
            EditValue::Nil => Value::Nil,
            EditValue::Bool(v) => Value::Bool(*v),
            EditValue::Number(v) => Value::Number(*v),
            EditValue::String(v) => Value::String(v.clone()),
            EditValue::Vector3(v) => Value::Vector3(*v),
            EditValue::Color3(v) => Value::Color3(*v),
            EditValue::UDim2(v) => Value::UDim2(*v),
            EditValue::Ref(v) => Value::Ref(v.map(|id| self.resolve(id)).transpose()?),
            EditValue::CFrame(v) => Value::CFrame(v.clone()),
        })
    }

    fn property(
        &mut self,
        editor: EditorId,
        target: (EditId, InstanceId),
        name: &str,
        value: &EditValue,
        index: usize,
        check_owner: bool,
    ) -> Result<(), EditError> {
        let (id, local) = target;
        if name == "Name" && self.world.is_builtin(local) {
            return Err(EditError::ProtectedBuiltin);
        }
        if check_owner
            && name == "Source"
            && self.is_script(local)
            && self.bindings.script_owners.get(&id) != Some(&editor)
        {
            return Err(EditError::ScriptNotOwned);
        }
        let value = self.value(value)?;
        self.world
            .set(local, name, value)
            .map_err(|source| EditError::Core {
                operation: index,
                source,
            })
    }

    fn apply(
        &mut self,
        editor: EditorId,
        batch: &EditBatch,
        limits: EditLimits,
        check_owner: bool,
    ) -> Result<(), EditError> {
        for (index, operation) in batch.operations.iter().enumerate() {
            match operation {
                EditOp::Create {
                    id,
                    class,
                    parent,
                    after,
                    properties,
                } => {
                    if id.editor != editor
                        || id.counter == 0
                        || id.counter <= self.bindings.creators.get(&editor).copied().unwrap_or(0)
                    {
                        return Err(EditError::CreationReuse);
                    }
                    if self.bindings.ids.len() >= limits.max_instances {
                        return Err(EditError::Limit("live instances"));
                    }
                    if !self.bindings.creators.contains_key(&editor)
                        && self
                            .bindings
                            .creators
                            .keys()
                            .filter(|key| key.0 != 0)
                            .count()
                            >= limits.max_clients
                    {
                        return Err(EditError::ClientLimit);
                    }
                    // Matches World service construction; other class/property rules stay in World.
                    if super::super::SERVICES.contains(&class.as_str())
                        || class == "StarterPlayerScripts"
                    {
                        return Err(EditError::ProtectedBuiltin);
                    }
                    let parent = self.parent(*parent)?;
                    let after = self.validate_anchor(*id, parent, *after)?;
                    let local =
                        self.world
                            .create(class, parent)
                            .map_err(|source| EditError::Core {
                                operation: index,
                                source,
                            })?;
                    self.bindings.ids.insert(*id, local);
                    self.bindings.reverse.insert(local, *id);
                    self.bindings.creators.insert(editor, id.counter);
                    if self.is_script(local) {
                        self.bindings.script_owners.insert(*id, editor);
                    }
                    self.order_after(local, parent, after, index)?;
                    for (name, value) in properties {
                        self.property(editor, (*id, local), name, value, index, check_owner)?;
                    }
                }
                EditOp::Set { id, properties } => {
                    let local = self.resolve(*id)?;
                    for (name, value) in properties {
                        self.property(editor, (*id, local), name, value, index, check_owner)?;
                    }
                }
                EditOp::Move { id, parent, after } => {
                    let local = self.resolve(*id)?;
                    self.subtree(local)?;
                    let parent = self.parent(*parent)?;
                    let after = self.validate_anchor(*id, parent, *after)?;
                    self.order_after(local, parent, after, index)?;
                }
                EditOp::Destroy { id } => {
                    let local = self.resolve(*id)?;
                    let gone = self.subtree(local)?;
                    let stable: Result<Vec<_>, _> = gone
                        .iter()
                        .map(|local| self.id_of(*local).ok_or(EditError::InvalidDocument))
                        .collect();
                    let stable = stable?;
                    self.world.destroy(local);
                    for (local, stable) in gone.into_iter().zip(stable) {
                        self.bindings.reverse.remove(&local);
                        self.bindings.ids.remove(&stable);
                        self.bindings.script_owners.remove(&stable);
                    }
                }
                EditOp::ScriptText { id, text } => {
                    let local = self.resolve(*id)?;
                    if !self.is_script(local) {
                        return Err(EditError::NotScript);
                    }
                    self.property(
                        editor,
                        (*id, local),
                        "Source",
                        &EditValue::String(text.clone()),
                        index,
                        check_owner,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn stable_value(&self, value: Value) -> Result<EditValue, EditError> {
        Ok(match value {
            Value::Nil => EditValue::Nil,
            Value::Bool(v) => EditValue::Bool(v),
            Value::Number(v) => EditValue::Number(v),
            Value::String(v) => EditValue::String(v),
            Value::Vector3(v) => EditValue::Vector3(v),
            Value::Color3(v) => EditValue::Color3(v),
            Value::UDim2(v) => EditValue::UDim2(v),
            Value::Ref(v) => EditValue::Ref(
                v.map(|id| self.id_of(id).ok_or(EditError::InvalidDocument))
                    .transpose()?,
            ),
            Value::CFrame(v) => EditValue::CFrame(v),
        })
    }

    pub(super) fn snapshot(&self) -> Result<EditSnapshot, EditError> {
        let roots = self
            .world
            .roots()
            .iter()
            .map(|id| self.id_of(*id).ok_or(EditError::InvalidDocument))
            .collect::<Result<_, _>>()?;
        let mut nodes = Vec::with_capacity(self.bindings.ids.len());
        for (&id, &local) in &self.bindings.ids {
            let class = self
                .world
                .class_name(local)
                .ok_or(EditError::InvalidDocument)?;
            let parent = self
                .world
                .parent(local)
                .map(|id| self.id_of(id).ok_or(EditError::InvalidDocument))
                .transpose()?;
            let children = self
                .world
                .children(local)
                .iter()
                .map(|id| self.id_of(*id).ok_or(EditError::InvalidDocument))
                .collect::<Result<_, _>>()?;
            let mut properties = BTreeMap::new();
            for property in classes::all_properties(class) {
                let value = self
                    .world
                    .get(local, property.name)
                    .map_err(|_| EditError::InvalidDocument)?;
                properties.insert(property.name.to_owned(), self.stable_value(value)?);
            }
            nodes.push(EditNode {
                id,
                class: class.to_owned(),
                parent,
                children,
                properties,
            });
        }
        Ok(EditSnapshot { roots, nodes })
    }
}

pub(super) fn validate_limits(limits: EditLimits) -> Result<(), EditError> {
    if limits.max_clients == 0
        || limits.max_operations == 0
        || limits.max_batch_bytes == 0
        || limits.max_script_bytes == 0
        || limits.max_receipts == 0
        || limits.max_receipt_bytes < limits.max_batch_bytes
        || limits.max_instances == 0
    {
        return Err(EditError::InvalidLimits);
    }
    Ok(())
}

pub(super) fn measure(batch: &EditBatch, limits: EditLimits) -> Result<usize, EditError> {
    if batch.operations.is_empty() || batch.operations.len() > limits.max_operations {
        return Err(EditError::Limit("operations"));
    }
    let mut bytes = std::mem::size_of::<EditBatch>();
    let mut add = |amount: usize| -> Result<(), EditError> {
        bytes = bytes
            .checked_add(amount)
            .ok_or(EditError::Limit("batch bytes"))?;
        if bytes > limits.max_batch_bytes {
            return Err(EditError::Limit("batch bytes"));
        }
        Ok(())
    };
    add(batch
        .operations
        .len()
        .checked_mul(std::mem::size_of::<EditOp>())
        .ok_or(EditError::Limit("batch bytes"))?)?;
    for operation in &batch.operations {
        let properties = match operation {
            EditOp::Create {
                class, properties, ..
            } => {
                add(class.len())?;
                Some(properties)
            }
            EditOp::Set { properties, .. } => Some(properties),
            EditOp::ScriptText { text, .. } => {
                if text.len() > limits.max_script_bytes {
                    return Err(EditError::Limit("script bytes"));
                }
                add(text.len())?;
                None
            }
            EditOp::Move { .. } | EditOp::Destroy { .. } => None,
        };
        if let Some(properties) = properties {
            add(properties
                .len()
                .checked_mul(std::mem::size_of::<(String, EditValue)>())
                .ok_or(EditError::Limit("batch bytes"))?)?;
            for (name, value) in properties {
                add(name.len())?;
                if let EditValue::CFrame(_) = value {
                    // The Box pointer is included in EditValue's native metadata; count its owned payload too.
                    add(std::mem::size_of::<crate::CFrame>())?;
                }
                if let EditValue::String(text) = value {
                    if name == "Source" && text.len() > limits.max_script_bytes {
                        return Err(EditError::Limit("script bytes"));
                    }
                    add(text.len())?;
                }
            }
        }
    }
    Ok(bytes)
}
