//! CPU side of the renderer: turns the parts of a [`World`] into draw lists.
use crate::materials::{material_index, material_params};
use crate::meshes::MeshKind;
use bytemuck::{Pod, Zeroable};
use glam::{EulerRot, Mat4, Vec3};
use metallic_core::{ChangeCursor, Changes, InstanceId, Value, World};
use std::collections::HashMap;

/// Per-instance data uploaded to the GPU (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceRaw {
    /// Model matrix (column major): translation * rotation * scale.
    pub model: [[f32; 4]; 4],
    /// Linear RGB colour and alpha (`1 - Transparency`).
    pub color: [f32; 4],
    /// Roughness, metalness, material id, unused.
    pub material: [f32; 4],
}

/// One part ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct PartDraw {
    /// The instance this was read from.
    pub id: InstanceId,
    /// Which shared mesh to draw.
    pub kind: MeshKind,
    /// GPU data.
    pub instance: InstanceRaw,
}

impl PartDraw {
    /// Alpha below 1 means the part goes to the blended pass.
    pub fn is_transparent(&self) -> bool {
        self.instance.color[3] < 0.999
    }

    /// True when the part is so transparent that drawing it would change nothing.
    pub fn is_invisible(&self) -> bool {
        self.instance.color[3] < 0.004
    }

    /// World position of the part's centre.
    pub fn centre(&self) -> Vec3 {
        Vec3::new(
            self.instance.model[3][0],
            self.instance.model[3][1],
            self.instance.model[3][2],
        )
    }
}

/// The parts of a world as the renderer sees them.
///
/// After the first full read, [`Scene::sync`] only re-reads the instances named by the world's
/// change journal.
#[derive(Default)]
pub struct Scene {
    slots: Vec<Option<PartDraw>>,
    free: Vec<usize>,
    index: HashMap<InstanceId, usize>,
    cursor: Option<ChangeCursor>,
    version: u64,
    full_rebuilds: u64,
}

impl Scene {
    /// An empty scene.
    pub fn new() -> Scene {
        Scene::default()
    }

    /// Brings the scene up to date with `world`. The first call (and any call after the
    /// journal overflowed or a different world was passed) reads every part under `Workspace`
    /// (under every root when the world has no `Workspace`); later calls re-read only changed
    /// instances. Returns true when anything changed; [`Scene::version`] then increases.
    pub fn sync(&mut self, world: &World) -> bool {
        metallic_profiling::backend_scope!("render.scene_sync");
        let journal = self.cursor.map(|c| world.changes_since(c));
        match journal {
            Some((Changes::Ids(ids), cursor)) => {
                let mut changed = false;
                for id in ids {
                    changed |= self.update(world, id);
                }
                self.cursor = Some(cursor);
                if changed {
                    self.version += 1;
                }
                changed
            }
            _ => {
                self.rebuild(world);
                true
            }
        }
    }

    fn rebuild(&mut self, world: &World) {
        self.slots.clear();
        self.free.clear();
        self.index.clear();
        for part in collect_parts(world) {
            self.index.insert(part.id, self.slots.len());
            self.slots.push(Some(part));
        }
        self.cursor = Some(world.change_cursor());
        self.version += 1;
        self.full_rebuilds += 1;
    }

    /// Re-reads one instance; true when the scene changed.
    fn update(&mut self, world: &World, id: InstanceId) -> bool {
        let want = if world.is_a(id, "BasePart") && in_render_tree(world, id) {
            read_part(world, id)
        } else {
            None
        };
        match (self.index.get(&id).copied(), want) {
            (Some(slot), Some(part)) => {
                if self.slots[slot].as_ref() == Some(&part) {
                    false
                } else {
                    self.slots[slot] = Some(part);
                    true
                }
            }
            (Some(slot), None) => {
                self.slots[slot] = None;
                self.free.push(slot);
                self.index.remove(&id);
                true
            }
            (None, Some(part)) => {
                let slot = self.free.pop().unwrap_or_else(|| {
                    self.slots.push(None);
                    self.slots.len() - 1
                });
                self.index.insert(id, slot);
                self.slots[slot] = Some(part);
                true
            }
            (None, None) => false,
        }
    }

    /// Increases whenever [`Scene::sync`] found a change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// How many times the whole world was read (first sync, journal overflow).
    pub fn full_rebuilds(&self) -> u64 {
        self.full_rebuilds
    }

    /// Number of parts, drawn or not.
    pub fn instance_count(&self) -> usize {
        self.index.len()
    }

    fn parts(&self) -> impl Iterator<Item = &PartDraw> {
        self.slots.iter().flatten()
    }

    /// Visible, fully opaque parts of one shape (the shared instanced pass).
    pub fn opaque(&self, kind: MeshKind) -> impl Iterator<Item = &PartDraw> {
        self.parts()
            .filter(move |p| p.kind == kind && !p.is_transparent())
    }

    /// Visible, partly transparent parts (the blended pass), unsorted.
    pub fn transparent(&self) -> impl Iterator<Item = &PartDraw> {
        self.parts()
            .filter(|p| p.is_transparent() && !p.is_invisible())
    }

    /// Fills `out` with the blended parts ordered far to near as seen from `eye`
    /// (clears it first, so one buffer can be reused every frame).
    pub fn transparent_sorted_into<'a>(&'a self, eye: Vec3, out: &mut Vec<&'a PartDraw>) {
        out.clear();
        out.extend(self.transparent());
        out.sort_by(|a, b| {
            let da = a.centre().distance_squared(eye);
            let db = b.centre().distance_squared(eye);
            db.total_cmp(&da)
        });
    }

    /// The blended parts ordered far to near as seen from `eye`.
    pub fn transparent_sorted(&self, eye: Vec3) -> Vec<&PartDraw> {
        let mut list = Vec::new();
        self.transparent_sorted_into(eye, &mut list);
        list
    }
}

/// True when the instance is drawn: under `Workspace`, or anywhere if the world has none.
fn in_render_tree(world: &World, id: InstanceId) -> bool {
    let Some(ws) = world.root("Workspace") else {
        return true;
    };
    let mut cur = Some(id);
    while let Some(c) = cur {
        if c == ws {
            return true;
        }
        cur = world.parent(c);
    }
    false
}

fn collect_parts(world: &World) -> Vec<PartDraw> {
    let mut out = Vec::new();
    let roots: Vec<InstanceId> = match world.root("Workspace") {
        Some(w) => vec![w],
        None => world.roots().to_vec(),
    };
    let mut stack: Vec<InstanceId> = roots.into_iter().rev().collect();
    while let Some(id) = stack.pop() {
        if world.is_a(id, "BasePart")
            && let Some(p) = read_part(world, id)
        {
            out.push(p);
        }
        stack.extend(world.children(id).iter().rev());
    }
    out
}

fn vec3_prop(world: &World, id: InstanceId, name: &str, default: Vec3) -> Vec3 {
    match world.get(id, name) {
        Ok(Value::Vector3(v)) => Vec3::new(v.x as f32, v.y as f32, v.z as f32),
        _ => default,
    }
}

fn read_part(world: &World, id: InstanceId) -> Option<PartDraw> {
    let position = vec3_prop(world, id, "Position", Vec3::ZERO);
    let rot = vec3_prop(world, id, "Orientation", Vec3::ZERO);
    let size = vec3_prop(world, id, "Size", Vec3::ONE).max(Vec3::splat(0.001));
    let color = match world.get(id, "Color") {
        Ok(Value::Color3(c)) => [c.r as f32, c.g as f32, c.b as f32],
        _ => [163.0, 162.0, 165.0],
    };
    let transparency = match world.get(id, "Transparency") {
        Ok(Value::Number(t)) => (t as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let material = match world.get(id, "Material") {
        Ok(Value::String(s)) => s,
        _ => String::from("Plastic"),
    };
    let shape = match world.get(id, "Shape") {
        Ok(Value::String(s)) => s,
        _ => String::from("Block"),
    };
    // Roblox applies Orientation as Y, then X, then Z.
    let rotation = Mat4::from_euler(
        EulerRot::YXZ,
        rot.y.to_radians(),
        rot.x.to_radians(),
        rot.z.to_radians(),
    );
    let model = Mat4::from_translation(position) * rotation * Mat4::from_scale(size);
    let params = material_params(&material);
    Some(PartDraw {
        id,
        kind: MeshKind::from_shape(&shape),
        instance: InstanceRaw {
            model: model.to_cols_array_2d(),
            color: [
                srgb_to_linear(color[0] / 255.0),
                srgb_to_linear(color[1] / 255.0),
                srgb_to_linear(color[2] / 255.0),
                1.0 - transparency,
            ],
            material: [
                params.roughness,
                params.metalness,
                material_index(&material) as f32,
                0.0,
            ],
        },
    })
}

/// Converts one sRGB encoded channel (0..=1) to linear light.
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::Vec3 as CoreVec3;

    fn world_with_parts() -> (World, Vec<InstanceId>) {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            let p = w.create("Part", Some(ws)).unwrap();
            w.set(
                p,
                "Position",
                Value::Vector3(CoreVec3::new(i as f64 * 10.0, 0.0, 0.0)),
            )
            .unwrap();
            ids.push(p);
        }
        w.create("Folder", Some(ws)).unwrap();
        (w, ids)
    }

    #[test]
    fn instance_buffer_matches_part_count() {
        let (w, _) = world_with_parts();
        let mut scene = Scene::new();
        assert!(scene.sync(&w));
        assert_eq!(scene.instance_count(), 3);
        let boxes = scene.opaque(MeshKind::Box).count();
        assert_eq!(boxes, 3);
        // A second sync with no change reports nothing new.
        let v = scene.version();
        assert!(!scene.sync(&w));
        assert_eq!(scene.version(), v);
    }

    #[test]
    fn transparent_parts_go_to_blended_pass() {
        let (mut w, ids) = world_with_parts();
        w.set(ids[1], "Transparency", Value::Number(0.5)).unwrap();
        let mut scene = Scene::new();
        scene.sync(&w);
        assert_eq!(scene.opaque(MeshKind::Box).count(), 2);
        let blended: Vec<_> = scene.transparent().collect();
        assert_eq!(blended.len(), 1);
        assert_eq!(blended[0].id, ids[1]);
        assert!((blended[0].instance.color[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blended_parts_are_sorted_far_to_near() {
        let (mut w, ids) = world_with_parts();
        for id in &ids {
            w.set(*id, "Transparency", Value::Number(0.3)).unwrap();
        }
        let mut scene = Scene::new();
        scene.sync(&w);
        let order: Vec<InstanceId> = scene
            .transparent_sorted(Vec3::new(-50.0, 0.0, 0.0))
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(order, vec![ids[2], ids[1], ids[0]]);
    }

    #[test]
    fn property_changes_and_destroys_are_picked_up() {
        let (mut w, ids) = world_with_parts();
        let mut scene = Scene::new();
        scene.sync(&w);
        w.set(
            ids[0],
            "Color",
            Value::Color3(metallic_core::Color3::new(255.0, 0.0, 0.0)),
        )
        .unwrap();
        assert!(scene.sync(&w));
        let red = scene.opaque(MeshKind::Box).next().unwrap().instance.color;
        assert!((red[0] - 1.0).abs() < 1e-6 && red[1] == 0.0);
        w.destroy(ids[2]);
        assert!(scene.sync(&w));
        assert_eq!(scene.instance_count(), 2);
    }

    #[test]
    fn shape_and_size_reach_the_instance() {
        let (mut w, ids) = world_with_parts();
        w.set(ids[0], "Shape", Value::String("Ball".into()))
            .unwrap();
        w.set(ids[0], "Size", Value::Vector3(CoreVec3::new(2.0, 2.0, 2.0)))
            .unwrap();
        let mut scene = Scene::new();
        scene.sync(&w);
        let ball = scene.opaque(MeshKind::Sphere).next().unwrap();
        assert_eq!(ball.id, ids[0]);
        assert_eq!(ball.instance.model[0][0], 2.0);
    }

    #[test]
    fn later_syncs_are_incremental() {
        let (mut w, ids) = world_with_parts();
        let mut scene = Scene::new();
        scene.sync(&w);
        assert_eq!(scene.full_rebuilds(), 1);
        // Move a part, add one, remove one, reparent one out of Workspace.
        w.set(
            ids[0],
            "Position",
            Value::Vector3(CoreVec3::new(0.0, 9.0, 0.0)),
        )
        .unwrap();
        let ws = w.root("Workspace").unwrap();
        let extra = w.create("Part", Some(ws)).unwrap();
        w.destroy(ids[1]);
        let storage = w.root("ReplicatedStorage").unwrap();
        w.set_parent(ids[2], Some(storage)).unwrap();
        assert!(scene.sync(&w));
        assert_eq!(scene.full_rebuilds(), 1, "no full rebuild");
        let mut present: Vec<InstanceId> = scene.opaque(MeshKind::Box).map(|p| p.id).collect();
        present.sort();
        let mut want = vec![ids[0], extra];
        want.sort();
        assert_eq!(present, want);
        // Back into Workspace: it appears again.
        w.set_parent(ids[2], Some(ws)).unwrap();
        assert!(scene.sync(&w));
        assert_eq!(scene.instance_count(), 3);
    }

    #[test]
    fn journal_overflow_falls_back_to_full_rebuild() {
        let (mut w, ids) = world_with_parts();
        let mut scene = Scene::new();
        scene.sync(&w);
        for i in 0..40_000 {
            let t = f64::from(i % 7) / 10.0 + 0.01;
            w.set(ids[0], "Transparency", Value::Number(t)).unwrap();
        }
        assert!(scene.sync(&w));
        assert_eq!(scene.full_rebuilds(), 2);
        assert_eq!(scene.instance_count(), 3);
    }

    /// Timing of the incremental path: 5,000 parts, 50 changed. Asserted (< 1 ms) in release only.
    #[test]
    fn sync_of_5000_parts_with_50_changed_is_fast() {
        let mut w = World::new_with_services();
        let ws = w.root("Workspace").unwrap();
        let ids: Vec<InstanceId> = (0..5000)
            .map(|_| w.create("Part", Some(ws)).unwrap())
            .collect();
        let mut scene = Scene::new();
        scene.sync(&w);
        let mut best = std::time::Duration::MAX;
        for round in 0..20 {
            for id in ids.iter().take(50) {
                let p = CoreVec3::new(f64::from(round) + 1.0, 0.0, 0.0);
                w.set(*id, "Position", Value::Vector3(p)).unwrap();
            }
            let t = std::time::Instant::now();
            assert!(scene.sync(&w));
            best = best.min(t.elapsed());
        }
        println!("sync 5000 parts / 50 changed: best of 20 = {best:?}");
        assert_eq!(scene.full_rebuilds(), 1);
        if !cfg!(debug_assertions) {
            assert!(best < std::time::Duration::from_millis(1), "took {best:?}");
        }
    }

    #[test]
    fn srgb_conversion_hits_the_ends() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
    }
}
