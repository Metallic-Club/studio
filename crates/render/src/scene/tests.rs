//! The CPU side of the scene against real games (`metallic_classes`): slots, packing, regions, coalescing.

use metallic_core::glam::{Quat, Vec3};
use metallic_core::{CFrame, Color3, DataModel, EnumItem, Id, PropId, Value};

use super::*;

struct Game {
    dm: DataModel,
    workspace: Id,
    part: metallic_core::ClassId,
    folder: metallic_core::ClassId,
    cframe: PropId,
}

impl Game {
    fn new() -> Self {
        let dm = metallic_classes::new_game().expect("a game");
        let registry = dm.registry().clone();
        let workspace = dm
            .service(registry.class_id("Workspace").expect("Workspace"))
            .expect("a Workspace");
        let part = registry.class_id("Part").expect("Part");
        let cframe = registry.prop_id(part, "CFrame").expect("CFrame");
        Self {
            dm,
            workspace,
            part,
            folder: registry.class_id("Folder").expect("Folder"),
            cframe,
        }
    }

    fn scene(&mut self) -> InstanceScene {
        InstanceScene::attach(&mut self.dm, Some(self.workspace), &RenderComponents::builtin()).expect("a consumer")
    }

    fn part_in(&mut self, parent: Option<Id>) -> Id {
        let id = self.dm.create(self.part).expect("a part");
        self.dm.set_parent(id, parent).expect("parented");
        id
    }

    fn part(&mut self) -> Id {
        self.part_in(Some(self.workspace))
    }

    fn move_to(&mut self, id: Id, x: f32) {
        self.dm
            .set(id, self.cframe, Value::CFrame(CFrame::new(Vec3::new(x, 0.0, 0.0))))
            .expect("moved");
    }
}

/// Runs a sync and a coalesce, as a frame does.
fn frame(scene: &mut InstanceScene, dm: &mut DataModel) -> SyncStats {
    let stats = scene.sync(dm);
    scene.coalesce();
    stats
}

fn instance(scene: &InstanceScene, id: Id) -> GpuInstance {
    let slot = scene.slot_of(id).expect("a slot");
    scene.instances(slot.region)[slot.slot as usize]
}

#[test]
fn a_part_is_packed_from_its_properties() {
    let mut game = Game::new();
    let id = game.part();
    let dm = &mut game.dm;
    let turned = CFrame::from_parts(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_z(0.5));
    dm.set_by_name(id, "CFrame", Value::CFrame(turned)).unwrap();
    dm.set_by_name(id, "Size", Value::Vector3(Vec3::new(2.0, 4.0, 6.0)))
        .unwrap();
    dm.set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(10, 20, 30)))
        .unwrap();
    dm.set_by_name(id, "Transparency", Value::Number(0.25)).unwrap();
    dm.set_by_name(id, "Material", Value::Enum(EnumItem::new("Material", "Metal")))
        .unwrap();
    dm.set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", "Cylinder")))
        .unwrap();
    dm.set_by_name(id, "CastShadow", Value::Bool(false)).unwrap();

    let mut scene = game.scene();
    let stats = frame(&mut scene, &mut game.dm);
    assert_eq!(stats.added, 1);
    let got = instance(&scene, id);
    let expected = GpuInstance {
        // A cylinder's diameter is the smaller of Y and Z.
        rows: affine_rows(turned, Vec3::new(2.0, 4.0, 4.0), Vec3::ZERO),
        color: u32::from_le_bytes([10, 20, 30, 0]),
        transparency: 0.25,
        // Metal is the third `Material` item.
        mesh_material: (u32::from(MeshKind::CYLINDER.0) << 16) | 2,
        flags: GpuInstance::LIVE | GpuInstance::TRANSLUCENT,
    };
    assert_eq!(got, expected);
}

#[test]
fn defaults_pack_a_grey_plastic_block_that_casts_shadows() {
    let mut game = Game::new();
    let id = game.part();
    let mut scene = game.scene();
    frame(&mut scene, &mut game.dm);
    let got = instance(&scene, id);
    assert_eq!(got.mesh(), MeshKind::BLOCK);
    assert_eq!(got.material_row(), 0);
    assert_eq!(got.flags, GpuInstance::LIVE | GpuInstance::CAST_SHADOW);
    assert_eq!(got.color, pack_color(Color3::new(0.64, 0.64, 0.64)));
    assert_eq!(
        got.rows,
        affine_rows(CFrame::IDENTITY, Vec3::new(4.0, 1.0, 2.0), Vec3::ZERO)
    );
}

#[test]
fn only_parts_in_the_world_are_drawn() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let loose = game.part_in(None);
    let folder = game.dm.create(game.folder).unwrap();
    let in_folder = game.part_in(Some(folder));
    let drawn = game.part();
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.slot_of(loose), None);
    assert_eq!(scene.slot_of(in_folder), None);
    assert!(scene.slot_of(drawn).is_some());
    // The camera and services are not renderable.
    assert_eq!(scene.live(Region::Static), 1);

    // Moving the folder into the world brings its part; moving it out takes it away again.
    game.dm.set_parent(folder, Some(game.workspace)).unwrap();
    let stats = frame(&mut scene, &mut game.dm);
    assert_eq!(stats.added, 1);
    assert!(scene.slot_of(in_folder).is_some());
    game.dm.set_parent(folder, None).unwrap();
    let stats = frame(&mut scene, &mut game.dm);
    assert_eq!(stats.removed, 1);
    assert_eq!(scene.slot_of(in_folder), None);
    assert_eq!(scene.live(Region::Static), 1);
}

#[test]
fn destroyed_parts_free_their_slot_for_the_next_one() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let parts: Vec<Id> = (0..4).map(|_| game.part()).collect();
    frame(&mut scene, &mut game.dm);
    let freed = scene.slot_of(parts[1]).unwrap();

    game.dm.destroy(parts[1]).unwrap();
    let stats = frame(&mut scene, &mut game.dm);
    assert_eq!(stats.removed, 1);
    assert_eq!(scene.slot_of(parts[1]), None);
    assert_eq!(scene.instances(Region::Static)[freed.slot as usize], GpuInstance::FREE);
    // Only the freed slot is uploaded.
    assert_eq!(
        scene.uploads(),
        [UploadRange {
            region: Region::Static,
            first: freed.slot,
            count: 1
        }]
    );

    let next = game.part();
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.slot_of(next), Some(freed), "the hole is filled first");
    assert_eq!(scene.instances(Region::Static).len(), 4, "nothing grew or compacted");
    for (i, &id) in parts.iter().enumerate().filter(|&(i, _)| i != 1) {
        assert_eq!(scene.slot_of(id).map(|s| s.slot), Some(i as u32));
    }
}

/// A small deterministic generator for the stability test.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn an_unchanged_part_keeps_its_slot_through_a_thousand_edits_elsewhere() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let mut others: Vec<Id> = (0..64).map(|_| game.part()).collect();
    let watched = game.part();
    others.extend((0..64).map(|_| game.part()));
    frame(&mut scene, &mut game.dm);
    let slot = scene.slot_of(watched).unwrap();
    let packed = instance(&scene, watched);

    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for edit in 0..1000 {
        match rng.below(4) {
            0 => others.push(game.part()),
            1 if others.len() > 1 => {
                let gone = others.swap_remove(rng.below(others.len()));
                game.dm.destroy(gone).unwrap();
            }
            2 => {
                let id = others[rng.below(others.len())];
                game.move_to(id, edit as f32);
            }
            _ => {
                let id = others[rng.below(others.len())];
                let colour = Color3::from_rgb_u8(rng.below(256) as u8, 0, 0);
                game.dm.set_by_name(id, "Color", Value::Color3(colour)).unwrap();
            }
        }
        if edit % 7 == 0 {
            frame(&mut scene, &mut game.dm);
            assert_eq!(scene.slot_of(watched), Some(slot), "after edit {edit}");
        }
    }
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.slot_of(watched), Some(slot));
    assert_eq!(instance(&scene, watched), packed);
    // Every live part has exactly one slot, and every slot's owner points back at it.
    for region in Region::ALL {
        for (slot, owner) in scene.owners(region).enumerate() {
            if let Some(owner) = owner {
                assert_eq!(
                    scene.slot_of(owner),
                    Some(SlotRef {
                        region,
                        slot: slot as u32
                    })
                );
            }
        }
    }
    assert_eq!(
        scene.live(Region::Static) + scene.live(Region::Dynamic),
        others.len() as u32 + 1
    );
}

#[test]
fn adjacent_changes_upload_as_one_range_and_scattered_ones_stay_bounded() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let parts: Vec<Id> = (0..1000).map(|_| game.part()).collect();
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.uploads().len(), 1, "the first sync uploads everything at once");

    // 100 adjacent parts change colour: one range.
    for &id in &parts[200..300] {
        game.dm
            .set_by_name(id, "Color", Value::Color3(Color3::new(1.0, 0.0, 0.0)))
            .unwrap();
    }
    frame(&mut scene, &mut game.dm);
    assert_eq!(
        scene.uploads(),
        [UploadRange {
            region: Region::Static,
            first: 200,
            count: 100
        }]
    );

    // Every tenth part: too far apart to merge, one range each and no more.
    for &id in parts.iter().step_by(10) {
        game.dm
            .set_by_name(id, "Color", Value::Color3(Color3::new(0.0, 1.0, 0.0)))
            .unwrap();
    }
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.uploads().len(), 100);
    assert!(scene.uploads().iter().all(|r| r.count == 1));

    // Every third part: the gaps are small enough to merge into one range.
    for &id in parts.iter().step_by(3) {
        game.dm
            .set_by_name(id, "Color", Value::Color3(Color3::new(0.0, 0.0, 1.0)))
            .unwrap();
    }
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.uploads().len(), 1);

    // Nothing changed: nothing to upload.
    frame(&mut scene, &mut game.dm);
    assert!(scene.uploads().is_empty());
}

#[test]
fn moving_parts_are_promoted_to_the_dynamic_region_and_demoted_at_rest() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let still = game.part();
    let falling = game.part();
    let anchored = game.part();
    game.dm.set_by_name(anchored, "Anchored", Value::Bool(true)).unwrap();
    frame(&mut scene, &mut game.dm);
    let still_slot = scene.slot_of(still).unwrap();

    // An unanchored part moving two frames running is promoted; an anchored one needs four.
    for step in 1..=PROMOTE_MOVES_ANCHORED {
        game.move_to(falling, f32::from(step));
        game.move_to(anchored, f32::from(step));
        let stats = frame(&mut scene, &mut game.dm);
        let expected = u32::from(step == PROMOTE_MOVES_UNANCHORED) + u32::from(step == PROMOTE_MOVES_ANCHORED);
        assert_eq!(stats.promoted, expected, "step {step}");
    }
    assert_eq!(scene.slot_of(falling).unwrap().region, Region::Dynamic);
    assert_eq!(scene.slot_of(anchored).unwrap().region, Region::Dynamic);
    assert_eq!(scene.slot_of(still), Some(still_slot));
    assert_eq!(instance(&scene, falling).translation(), Vec3::new(4.0, 0.0, 0.0));
    assert_eq!(scene.live(Region::Dynamic), 2);

    // After resting, both go back, with their packed data unchanged.
    let packed = instance(&scene, falling);
    let mut demoted = 0;
    for _ in 0..DEMOTE_REST + 2 {
        demoted += frame(&mut scene, &mut game.dm).demoted;
    }
    assert_eq!(demoted, 2);
    assert_eq!(scene.slot_of(falling).unwrap().region, Region::Static);
    assert_eq!(instance(&scene, falling), packed);
    assert_eq!(scene.live(Region::Dynamic), 0);
}

#[test]
fn moves_spread_wider_than_the_window_never_promote() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let slow = game.part();
    frame(&mut scene, &mut game.dm);
    for step in 0..4 {
        game.move_to(slow, step as f32);
        frame(&mut scene, &mut game.dm);
        for _ in 0..PROMOTE_WINDOW {
            frame(&mut scene, &mut game.dm);
        }
    }
    assert_eq!(scene.slot_of(slow).unwrap().region, Region::Static);
}

#[test]
fn the_origin_follows_the_camera_in_grid_steps() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let id = game.part();
    game.move_to(id, 5000.0);
    frame(&mut scene, &mut game.dm);
    assert_eq!(instance(&scene, id).translation(), Vec3::new(5000.0, 0.0, 0.0));

    // Within a grid step of the origin: nothing moves.
    assert!(!scene.set_camera_position(&game.dm, Vec3::new(ORIGIN_GRID, 0.0, -ORIGIN_GRID)));
    // Further: the origin snaps near the camera and every translation is re-made relative to it.
    assert!(scene.set_camera_position(&game.dm, Vec3::new(4900.0, 0.0, 0.0)));
    assert_eq!(scene.origin(), Vec3::new(5120.0, 0.0, 0.0));
    assert_eq!(instance(&scene, id).translation(), Vec3::new(-120.0, 0.0, 0.0));
    scene.coalesce();
    assert_eq!(scene.uploads().len(), 1);

    // Later changes pack relative to the new origin.
    game.move_to(id, 5100.0);
    frame(&mut scene, &mut game.dm);
    assert_eq!(instance(&scene, id).translation(), Vec3::new(-20.0, 0.0, 0.0));
}

#[test]
fn a_rebuild_places_everything_again() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let parts: Vec<Id> = (0..10).map(|_| game.part()).collect();
    frame(&mut scene, &mut game.dm);
    let before: Vec<GpuInstance> = parts.iter().map(|&id| instance(&scene, id)).collect();
    game.dm.destroy(parts[0]).unwrap();
    scene.rebuild_all(&mut game.dm);
    let stats = frame(&mut scene, &mut game.dm);
    assert_eq!(stats.added, 9);
    assert_eq!(scene.instances(Region::Static).len(), 9, "the rebuild packs densely");
    for (id, packed) in parts.iter().zip(&before).skip(1) {
        assert_eq!(instance(&scene, *id), *packed);
    }
}

#[test]
fn a_scene_ignores_other_data_models_and_frees_its_consumer() {
    let mut game = Game::new();
    let scene = game.scene();
    let mut other = Game::new();
    let mut scene2 = other.scene();
    other.part();
    assert_eq!(scene2.sync(&mut game.dm), SyncStats::default());
    assert_eq!(frame(&mut scene2, &mut other.dm).added, 1);
    // Detaching gives the consumer back: as many scenes as there are consumers can attach again.
    scene.detach(&mut game.dm);
    for _ in 0..8 {
        let again = game.scene();
        again.detach(&mut game.dm);
    }
}

/// Checks every draw list against a scan of the slots: each drawn slot is listed exactly once, in the list its
/// packed instance belongs in, and nothing else is listed.
fn assert_lists_match_slots(scene: &InstanceScene) {
    let mut expected: Vec<Vec<u32>> = vec![Vec::new(); usize::from(MeshKind::BUILTIN_COUNT)];
    let mut transparent = Vec::new();
    for region in Region::ALL {
        for (slot, instance) in scene.instances(region).iter().enumerate() {
            let e = entry(region, slot as u32);
            match lists::list_of(instance) {
                lists::NOT_LISTED => {}
                lists::TRANSPARENT_LIST => transparent.push(e),
                kind => expected[usize::from(kind)].push(e),
            }
        }
    }
    for (kind, want) in expected.iter_mut().enumerate() {
        let mut have = scene.draw_list(MeshKind(kind as u16)).to_vec();
        have.sort_unstable();
        want.sort_unstable();
        assert_eq!(&have, want, "the opaque list of kind {kind}");
    }
    let mut have = scene.transparent_list().to_vec();
    have.sort_unstable();
    transparent.sort_unstable();
    assert_eq!(have, transparent, "the transparent list");
}

#[test]
fn the_draw_lists_follow_every_kind_of_change() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let mut parts: Vec<Id> = (0..48).map(|_| game.part()).collect();
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.draw_list(MeshKind::BLOCK).len(), 48);
    assert_lists_match_slots(&scene);

    let shapes = ["Block", "Ball", "Cylinder", "Wedge"];
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    for edit in 0..3000 {
        match rng.below(7) {
            0 => parts.push(game.part()),
            1 if parts.len() > 1 => {
                let gone = parts.swap_remove(rng.below(parts.len()));
                game.dm.destroy(gone).unwrap();
            }
            2 => {
                let id = parts[rng.below(parts.len())];
                let shape = EnumItem::new("PartShape", shapes[rng.below(4)]);
                game.dm.set_by_name(id, "Shape", Value::Enum(shape)).unwrap();
            }
            3 => {
                let id = parts[rng.below(parts.len())];
                let transparency = [0.0, 0.5, 1.0][rng.below(3)];
                game.dm
                    .set_by_name(id, "Transparency", Value::Number(transparency))
                    .unwrap();
            }
            4 => {
                // Out of the world and back.
                let id = parts[rng.below(parts.len())];
                let parent = if rng.below(2) == 0 { None } else { Some(game.workspace) };
                game.dm.set_parent(id, parent).unwrap();
            }
            _ => {
                // The same few parts keep moving, so some are promoted and change region (and slot).
                let id = parts[rng.below(parts.len().min(8))];
                game.move_to(id, edit as f32);
            }
        }
        if edit % 5 == 0 {
            frame(&mut scene, &mut game.dm);
            assert_lists_match_slots(&scene);
        }
    }
    frame(&mut scene, &mut game.dm);
    assert_lists_match_slots(&scene);
    assert!(scene.live(Region::Dynamic) > 0, "nothing was promoted");

    scene.rebuild_all(&mut game.dm);
    frame(&mut scene, &mut game.dm);
    assert_lists_match_slots(&scene);
}

#[test]
fn only_transparent_changes_move_the_transparent_revision() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let solid = game.part();
    let glass = game.part();
    game.dm.set_by_name(glass, "Transparency", Value::Number(0.5)).unwrap();
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.transparent_list().len(), 1);
    let revision = scene.transparent_revision();
    game.move_to(solid, 3.0);
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.transparent_revision(), revision, "a solid part moved");
    game.move_to(glass, 3.0);
    frame(&mut scene, &mut game.dm);
    assert_ne!(scene.transparent_revision(), revision, "the see-through part moved");
}

/// Whether `level` is a level `view` may draw `instance` at: between the levels its projected size gives at the two
/// edges of the hysteresis band (crate::lod), and 0 for kinds without levels.
fn level_allowed(view: &crate::cull::CullView, instance: &GpuInstance, level: usize) -> bool {
    let class = crate::lod::lod_class(instance.mesh());
    if class == 0 || !view.lod.enabled() {
        return level == 0;
    }
    let (center, radius) = grid::bounding_sphere(instance);
    let d2 = (center - view.eye).length_squared();
    let finest = view.lod.choose(class, radius * radius, d2, 0);
    let coarsest = view.lod.choose(class, radius * radius, d2, crate::lod::COARSEST);
    (usize::from(finest)..=usize::from(coarsest)).contains(&level)
}

/// Checks a cull of `scene` against testing every listed instance's bounding sphere on its own: the same entries
/// in every list, each opaque level list in front-to-back bucket order, and every level one its size allows.
fn assert_cull_matches_brute_force(scene: &mut InstanceScene, view: &crate::cull::CullView) -> crate::cull::CullStats {
    let mut set = crate::cull::VisibleSet::new();
    let stats = scene.cull(view, &mut set);
    let visible = |e: &u32| {
        let instance = scene.entry_instance(*e).expect("a listed slot");
        let (center, radius) = grid::bounding_sphere(instance);
        view.sphere_visible(center, radius)
    };
    let mut total = 0;
    for kind in 0..scene.draw_list_count() {
        let kind = MeshKind(kind as u16);
        let mut want: Vec<u32> = scene.draw_list(kind).iter().copied().filter(visible).collect();
        let mut have = Vec::new();
        for level in 0..crate::lod::MAX_LOD_LEVELS {
            let listed = set.opaque(kind, level);
            // Front to back a grid cell (or large item) at a time: by the depth of the cell's box.
            let keys: Vec<u32> = listed
                .iter()
                .map(|e| {
                    let instance = scene.entry_instance(*e).expect("listed");
                    assert!(
                        level_allowed(view, instance, level),
                        "kind {kind:?} drawn at level {level}"
                    );
                    let (center, radius) = grid::bounding_sphere(instance);
                    if radius > grid::LARGE_RADIUS {
                        view.box_key(center, grid::half_extents(instance))
                    } else {
                        let cell = (grid::key_of(center).as_vec3() + 0.5) * grid::CELL_SIZE;
                        view.box_key(cell, Vec3::splat(grid::CELL_SIZE / 2.0))
                    }
                })
                .collect();
            assert!(
                keys.is_sorted(),
                "kind {kind:?} level {level} is not front to back: {keys:?}"
            );
            have.extend_from_slice(listed);
        }
        assert_eq!(have.len(), set.opaque_len(kind));
        have.sort_unstable();
        want.sort_unstable();
        assert_eq!(have, want, "the visible entries of kind {kind:?}");
        total += have.len();
    }
    let mut want: Vec<u32> = scene.transparent_list().iter().copied().filter(visible).collect();
    assert_eq!(set.transparent().len(), set.transparent_levels().len());
    for (&e, &level) in set.transparent().iter().zip(set.transparent_levels()) {
        let instance = scene.entry_instance(e).expect("listed");
        assert!(
            level_allowed(view, instance, usize::from(level)),
            "see-through {e} at level {level}"
        );
    }
    let mut have = set.transparent().to_vec();
    have.sort_unstable();
    want.sort_unstable();
    assert_eq!(have, want, "the visible see-through entries");
    total += have.len();
    assert_eq!(stats.visible as usize, total);
    let listed: usize = (0..scene.draw_list_count())
        .map(|k| scene.draw_list(MeshKind(k as u16)).len())
        .sum::<usize>()
        + scene.transparent_list().len();
    assert_eq!(stats.candidates as usize, listed);
    assert_eq!(scene.culling_items(), listed);
    stats
}

#[test]
fn a_cull_matches_testing_every_part_through_every_kind_of_change() {
    use crate::cull::{CullSettings, CullView};
    use crate::parts::Camera;

    let look = |eye: Vec3, target: Vec3, fov: f64| Camera::from_cframe(CFrame::look_at(eye, target, Vec3::Y), fov);
    let views = [
        // Along the row the parts are on, with a view distance and a screen size that both cut some off.
        CullView::for_camera(
            &look(Vec3::new(-20.0, 5.0, 0.0), Vec3::new(100.0, 0.0, 0.0), 70.0),
            Vec3::ZERO,
            640,
            360,
            &CullSettings {
                max_distance: 900.0,
                min_screen_size: 3.0,
                ..CullSettings::default()
            },
        ),
        // Across it, a narrow view.
        CullView::for_camera(
            &look(Vec3::new(500.0, 30.0, 80.0), Vec3::new(400.0, 0.0, 0.0), 30.0),
            Vec3::ZERO,
            1920,
            1080,
            &CullSettings::default(),
        ),
    ];
    let mut game = Game::new();
    let mut scene = game.scene();
    let mut parts: Vec<Id> = (0..64).map(|_| game.part()).collect();
    for (i, &id) in parts.iter().enumerate() {
        game.move_to(id, i as f32 * 25.0);
    }
    let shapes = ["Block", "Ball", "Cylinder", "Wedge"];
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    // Per view: visible, outside the frustum, too far or small, over every cull.
    let mut totals = [(0u32, 0u32, 0u32); 2];
    for edit in 0..2000 {
        match rng.below(6) {
            0 => parts.push(game.part()),
            1 if parts.len() > 1 => {
                let gone = parts.swap_remove(rng.below(parts.len()));
                game.dm.destroy(gone).unwrap();
            }
            2 => {
                let id = parts[rng.below(parts.len())];
                let shape = EnumItem::new("PartShape", shapes[rng.below(4)]);
                game.dm.set_by_name(id, "Shape", Value::Enum(shape)).unwrap();
            }
            3 => {
                let id = parts[rng.below(parts.len())];
                let transparency = [0.0, 0.5, 1.0][rng.below(3)];
                game.dm
                    .set_by_name(id, "Transparency", Value::Number(transparency))
                    .unwrap();
            }
            4 => {
                // The same few parts keep moving, so some are promoted and change region (and slot).
                let id = parts[rng.below(parts.len().min(8))];
                game.move_to(id, edit as f32 * 0.5);
            }
            _ => {
                // Others jump anywhere along the row, across cells.
                let id = parts[rng.below(parts.len())];
                game.move_to(id, rng.below(1500) as f32 - 200.0);
            }
        }
        if edit % 7 == 0 {
            frame(&mut scene, &mut game.dm);
            for (view, totals) in views.iter().zip(&mut totals) {
                let stats = assert_cull_matches_brute_force(&mut scene, view);
                totals.0 += stats.visible;
                totals.1 += stats.outside_frustum;
                totals.2 += stats.too_far_or_small;
            }
        }
    }
    // Each view kept some parts and culled others, the first one also for distance and size.
    assert!(totals.iter().all(|t| t.0 > 0 && t.1 > 0), "{totals:?}");
    assert!(totals[0].2 > 0, "{totals:?}");
    assert!(scene.live(Region::Dynamic) > 0, "nothing was promoted");
    scene.rebuild_all(&mut game.dm);
    frame(&mut scene, &mut game.dm);
    for view in &views {
        assert_cull_matches_brute_force(&mut scene, view);
    }
}

/// The level `set` lists the opaque `entry` of `kind` at, if it lists it.
fn listed_level(set: &crate::cull::VisibleSet, kind: MeshKind, entry: u32) -> Option<usize> {
    (0..crate::lod::MAX_LOD_LEVELS).find(|&level| set.opaque(kind, level).contains(&entry))
}

/// A camera on the X axis `distance` metres from the origin, looking at it.
fn camera_at(distance: f32) -> crate::cull::CullView {
    camera_towards(0.0, distance)
}

/// A camera on the X axis `distance` metres past `x`, looking back at `x`.
fn camera_towards(x: f32, distance: f32) -> crate::cull::CullView {
    let target = Vec3::new(x, 0.0, 0.0);
    let camera = crate::parts::Camera::from_cframe(CFrame::look_at(target + Vec3::X * distance, target, Vec3::Y), 70.0);
    crate::cull::CullView::for_camera(&camera, Vec3::ZERO, 1920, 1080, &crate::cull::CullSettings::default())
        .with_lod(&crate::lod::LodSettings::MEDIUM)
}

#[test]
fn a_ball_slowly_crossing_a_threshold_switches_once() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let ball = game.part();
    game.dm
        .set_by_name(ball, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .unwrap();
    game.dm
        .set_by_name(ball, "Size", Value::Vector3(Vec3::splat(2.0)))
        .unwrap();
    frame(&mut scene, &mut game.dm);
    let slot = scene.slot_of(ball).unwrap();
    let e = entry(slot.region, slot.slot);
    let (_, radius) = grid::bounding_sphere(&instance(&scene, ball));
    // Where the band between full detail and level 1 lies: a ball at full detail goes coarser past `coarser`, one at
    // level 1 comes back nearer than `finer`; the threshold is between.
    let view = camera_at(1.0);
    let edge = |previous: u8| {
        let (mut near, mut far) = (1.0f32, 10_000.0f32);
        for _ in 0..60 {
            let mid = (near + far) / 2.0;
            if view.lod.choose(1, radius * radius, mid * mid, previous) == 0 {
                near = mid;
            } else {
                far = mid;
            }
        }
        near
    };
    let (finer, coarser) = (edge(1), edge(0));
    assert!(finer < coarser);
    let threshold = (finer * coarser).sqrt();
    // 600 frames: the camera drifts from 5% inside the band's near edge to 5% past its far edge, shaking by 4% on
    // the way, so the threshold is crossed again and again.
    let mut set = crate::cull::VisibleSet::new();
    let (mut levels, mut crossings, mut last_side) = (Vec::new(), 0, None);
    for frame_index in 0..600 {
        let t = frame_index as f32 / 600.0;
        let shake = 1.0 + 0.04 * (frame_index as f32 * 0.9).sin();
        let d = (finer * 0.95 + (coarser * 1.05 - finer * 0.95) * t) * shake;
        scene.cull(&camera_at(d), &mut set);
        levels.push(listed_level(&set, MeshKind::BALL, e).expect("in view"));
        let side = d < threshold;
        if last_side.is_some_and(|s| s != side) {
            crossings += 1;
        }
        last_side = Some(side);
    }
    let switches = levels.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!((levels[0], levels[599]), (0, 1));
    assert_eq!(switches, 1, "{levels:?}");
    assert!(crossings > 10, "the drift crosses the threshold {crossings} times");
}

#[test]
fn a_moved_part_keeps_its_level() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let ball = game.part();
    game.dm
        .set_by_name(ball, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .unwrap();
    frame(&mut scene, &mut game.dm);
    let mut set = crate::cull::VisibleSet::new();
    let level_of = |scene: &mut InstanceScene, set: &mut crate::cull::VisibleSet, id: Id, (x, d): (f32, f32)| {
        scene.cull(&camera_towards(x, d), set);
        let slot = scene.slot_of(id).unwrap();
        listed_level(set, MeshKind::BALL, entry(slot.region, slot.slot))
    };
    // Walk in from far until the ball is drawn at full detail: that happens 10% past the threshold.
    assert_eq!(level_of(&mut scene, &mut set, ball, (0.0, 400.0)), Some(3));
    let mut d = 400.0;
    while level_of(&mut scene, &mut set, ball, (0.0, d)) != Some(0) {
        d *= 0.995;
    }
    // Stepping back out by 15% is past the plain threshold but inside the band: still full detail.
    let back = d * 1.15;
    let (_, radius) = grid::bounding_sphere(&instance(&scene, ball));
    let lod = camera_at(back).lod;
    assert_eq!(
        lod.choose(1, radius * radius, back * back, 1),
        1,
        "inside the band: a ball at level 1 would stay there"
    );
    assert_eq!(level_of(&mut scene, &mut set, ball, (0.0, back)), Some(0));
    // Moving the ball a little (inside its cell, then into another cell, which also promotes it to the dynamic
    // region: a new slot) keeps the level it was drawn at.
    game.move_to(ball, 0.25);
    frame(&mut scene, &mut game.dm);
    assert_eq!(level_of(&mut scene, &mut set, ball, (0.25, back)), Some(0));
    game.move_to(ball, -40.0);
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.slot_of(ball).unwrap().region, Region::Dynamic);
    assert_eq!(level_of(&mut scene, &mut set, ball, (-40.0, back)), Some(0));
    // A new ball at the same place starts at the coarse side of the band.
    let other = game.part();
    game.dm
        .set_by_name(other, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .unwrap();
    game.move_to(other, -40.0);
    frame(&mut scene, &mut game.dm);
    assert_eq!(level_of(&mut scene, &mut set, other, (-40.0, back)), Some(1));
    assert_eq!(level_of(&mut scene, &mut set, ball, (-40.0, back)), Some(0));
}

#[test]
fn levels_off_draw_everything_at_full_detail() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let shapes = ["Block", "Ball", "Cylinder", "Wedge"];
    for i in 0..40 {
        let id = game.part();
        game.dm
            .set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", shapes[i % 4])))
            .unwrap();
        game.move_to(id, i as f32 * -30.0);
    }
    frame(&mut scene, &mut game.dm);
    let view = camera_at(20.0);
    let mut set = crate::cull::VisibleSet::new();
    scene.cull(&view, &mut set);
    let coarse: usize = (1..crate::lod::MAX_LOD_LEVELS)
        .map(|level| set.opaque(MeshKind::BALL, level).len() + set.opaque(MeshKind::CYLINDER, level).len())
        .sum();
    assert!(coarse > 0, "far balls and cylinders are coarser");
    assert!(
        set.opaque(MeshKind::BALL, 3).len() > 0,
        "the farthest balls at the coarsest level"
    );
    let visible = set.len();
    scene.cull(&view.with_lod(&crate::lod::LodSettings::OFF), &mut set);
    assert_eq!(set.len(), visible, "the same parts");
    for kind in 0..4 {
        for level in 1..crate::lod::MAX_LOD_LEVELS {
            assert!(set.opaque(MeshKind(kind), level).is_empty());
        }
    }
}

#[test]
fn moves_queue_each_slot_once_until_a_cull_and_recolours_none() {
    let mut game = Game::new();
    let mut scene = game.scene();
    let mut parts: Vec<Id> = (0..32).map(|_| game.part()).collect();
    frame(&mut scene, &mut game.dm);
    let camera =
        crate::parts::Camera::from_cframe(CFrame::look_at(Vec3::new(0.0, 5.0, 20.0), Vec3::ZERO, Vec3::Y), 70.0);
    let view = crate::cull::CullView::for_camera(&camera, Vec3::ZERO, 640, 360, &crate::cull::CullSettings::default());
    scene.cull(&view, &mut crate::cull::VisibleSet::new());
    assert_eq!(scene.culling_queue(), 0);
    // Recolouring moves nothing.
    for &id in &parts {
        game.dm
            .set_by_name(id, "Color", Value::Color3(Color3::new(0.2, 0.4, 0.6)))
            .unwrap();
    }
    frame(&mut scene, &mut game.dm);
    assert_eq!(scene.culling_queue(), 0);
    // A hundred frames of moving, destroying and creating with no cull: every slot is queued at most once.
    for step in 0..100 {
        for (i, &id) in parts.iter().enumerate().take(8) {
            game.move_to(id, (step * 3 + i) as f32);
        }
        let gone = parts.swap_remove(step % parts.len());
        game.dm.destroy(gone).unwrap();
        let new = game.part();
        game.move_to(new, step as f32);
        parts.push(new);
        frame(&mut scene, &mut game.dm);
        let slots = scene.instances(Region::Static).len() + scene.instances(Region::Dynamic).len();
        assert!(
            scene.culling_queue() <= slots,
            "{} queued for {slots} slots",
            scene.culling_queue()
        );
    }
    assert_cull_matches_brute_force(&mut scene, &view);
    assert_eq!(scene.culling_queue(), 0);
}
