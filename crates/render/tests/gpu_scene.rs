//! The GPU scene on a real adapter: parts are created, changed and destroyed, the instance buffers are read back
//! and compared with the packed data, through growth, slot reuse and the dynamic region. Skips when the machine
//! has no adapter (see `support::gpu`).

mod support;

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, EnumItem, Id, PropId, Value};
use metallic_render::scene::{
    GpuInstance, InstanceBinding, MeshKind, PROMOTE_MOVES_UNANCHORED, Region, RenderComponents, UploadStats,
    affine_rows,
};
use metallic_render::{GpuScene, RenderContext, wgpu};
use support::{gpu, read_back_buffer};

struct Game {
    dm: DataModel,
    workspace: Id,
    part: metallic_core::ClassId,
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
            cframe,
        }
    }

    fn part(&mut self) -> Id {
        let id = self.dm.create(self.part).expect("a part");
        self.dm.set_parent(id, Some(self.workspace)).expect("parented");
        id
    }

    fn move_to(&mut self, id: Id, position: Vec3) {
        self.dm
            .set(id, self.cframe, Value::CFrame(CFrame::new(position)))
            .expect("moved");
    }
}

/// One frame: sync, then the upload submitted on its own.
fn frame(context: &RenderContext, scene: &mut GpuScene, dm: &mut DataModel) -> UploadStats {
    scene.sync(dm);
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let stats = scene.upload(&mut encoder);
    context.queue().submit(Some(encoder.finish()));
    stats
}

/// The GPU's copy of `region`, as many slots as the scene uses there.
fn gpu_instances(context: &RenderContext, scene: &GpuScene, region: Region) -> Vec<GpuInstance> {
    let len = scene.scene().instances(region).len() as u64;
    let bytes = read_back_buffer(context, scene.buffer(region), len * GpuInstance::SIZE);
    bytemuck::pod_collect_to_vec(&bytes)
}

/// Both regions on the GPU equal the scene's packed instances.
fn assert_gpu_matches(context: &RenderContext, scene: &GpuScene) {
    for region in Region::ALL {
        assert_eq!(
            gpu_instances(context, scene, region),
            scene.scene().instances(region),
            "{region:?} region"
        );
    }
}

fn new_scene(context: &RenderContext, game: &mut Game) -> GpuScene {
    GpuScene::new(
        context,
        &mut game.dm,
        Some(game.workspace),
        &RenderComponents::builtin(),
    )
    .expect("a scene")
}

#[test]
fn created_changed_and_destroyed_parts_reach_the_buffer() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    let parts: Vec<Id> = (0..3).map(|_| game.part()).collect();
    let mut scene = new_scene(&context, &mut game);
    let expected_binding = if context.capabilities().vertex_storage {
        InstanceBinding::Storage
    } else {
        InstanceBinding::Vertex
    };
    assert_eq!(scene.binding(), expected_binding);

    let stats = frame(&context, &mut scene, &mut game.dm);
    assert_eq!((stats.writes, stats.ranges, stats.slots), (1, 1, 3));
    assert_gpu_matches(&context, &scene);

    // Change one part's every visual property and its place.
    let id = parts[2];
    game.move_to(id, Vec3::new(1.0, 2.0, 3.0));
    let dm = &mut game.dm;
    dm.set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(255, 128, 0)))
        .unwrap();
    dm.set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
        .unwrap();
    dm.set_by_name(id, "Material", Value::Enum(EnumItem::new("Material", "Neon")))
        .unwrap();
    dm.set_by_name(id, "Transparency", Value::Number(1.0)).unwrap();
    let stats = frame(&context, &mut scene, &mut game.dm);
    assert_eq!((stats.ranges, stats.slots), (1, 1));
    let on_gpu = gpu_instances(&context, &scene, Region::Static);
    assert_eq!(
        on_gpu[2],
        GpuInstance {
            // A ball's diameter is the smallest side of the default 4 × 1 × 2.
            rows: affine_rows(CFrame::new(Vec3::new(1.0, 2.0, 3.0)), Vec3::ONE, Vec3::ZERO),
            color: u32::from_le_bytes([255, 128, 0, 0]),
            transparency: 1.0,
            mesh_material: (u32::from(MeshKind::BALL.0) << 16) | 5,
            flags: GpuInstance::LIVE | GpuInstance::CAST_SHADOW | GpuInstance::HIDDEN,
        }
    );

    // A destroyed part's slot is zeroed on the GPU, and the next part reuses it.
    game.dm.destroy(parts[0]).unwrap();
    frame(&context, &mut scene, &mut game.dm);
    assert_eq!(gpu_instances(&context, &scene, Region::Static)[0], GpuInstance::FREE);
    let next = game.part();
    frame(&context, &mut scene, &mut game.dm);
    assert_eq!(scene.scene().slot_of(next).map(|s| s.slot), Some(0));
    assert_gpu_matches(&context, &scene);
}

#[test]
fn buffers_grow_on_the_gpu_and_keep_their_contents() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    let mut scene = new_scene(&context, &mut game);
    let first: Vec<Id> = (0..1000).map(|_| game.part()).collect();
    frame(&context, &mut scene, &mut game.dm);
    let capacity = scene.capacity(Region::Static);
    let revision = scene.buffer_revision();

    // More parts than the buffer holds: it is replaced, the old slots copied on the GPU (only the new ones are
    // uploaded).
    for i in 0..2000 {
        let id = game.part();
        game.move_to(id, Vec3::new(i as f32, 0.0, 0.0));
    }
    let stats = frame(&context, &mut scene, &mut game.dm);
    assert_eq!(stats.grown, 1);
    assert_eq!(stats.slots, 2000);
    assert!(scene.capacity(Region::Static) >= 3000 && scene.capacity(Region::Static) > capacity);
    assert_ne!(scene.buffer_revision(), revision);
    assert_gpu_matches(&context, &scene);
    assert!(scene.scene().slot_of(first[999]).is_some());
}

#[test]
fn moving_parts_are_uploaded_to_the_dynamic_buffer() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    let parts: Vec<Id> = (0..200).map(|_| game.part()).collect();
    let mut scene = new_scene(&context, &mut game);
    frame(&context, &mut scene, &mut game.dm);

    // Every fourth part moves on consecutive frames: promoted, they sit together in the dynamic region, so each
    // frame's moves upload as one range there.
    let movers: Vec<Id> = parts.iter().copied().step_by(4).collect();
    let mut last = UploadStats::default();
    for step in 0..PROMOTE_MOVES_UNANCHORED + 3 {
        for (i, &id) in movers.iter().enumerate() {
            game.move_to(id, Vec3::new(i as f32, f32::from(step), 0.0));
        }
        last = frame(&context, &mut scene, &mut game.dm);
    }
    assert_eq!(scene.scene().live(Region::Dynamic), movers.len() as u32);
    assert_eq!((last.ranges, last.slots), (1, movers.len() as u32));
    assert_gpu_matches(&context, &scene);
    // The static slots the movers left are zeroed.
    let statics = gpu_instances(&context, &scene, Region::Static);
    assert_eq!(
        statics.iter().filter(|i| i.flags & GpuInstance::LIVE == 0).count(),
        movers.len()
    );
}

#[test]
fn the_scatter_pass_and_copies_upload_the_same() {
    let Some(context) = gpu() else { return };
    for scatter in [true, false] {
        let mut game = Game::new();
        let parts: Vec<Id> = (0..600).map(|_| game.part()).collect();
        let mut scene = new_scene(&context, &mut game);
        if scatter && !scene.can_scatter() {
            eprintln!("no scatter pass on this device");
            continue;
        }
        scene.set_scatter_min_ranges(if scatter { 0 } else { u32::MAX });
        let expect_dispatch = u32::from(scatter);
        let stats = frame(&context, &mut scene, &mut game.dm);
        assert_eq!((stats.dispatches, stats.copies), (expect_dispatch, 1 - expect_dispatch));
        assert_gpu_matches(&context, &scene);

        for step in 0..PROMOTE_MOVES_UNANCHORED + 2 {
            // Scattered recolours, movers headed for the dynamic region, removals and enough new parts to grow
            // the static buffer, all in the same frames.
            for &id in parts.iter().skip(usize::from(step)).step_by(7) {
                if !game.dm.contains(id) {
                    continue;
                }
                let colour = Color3::from_rgb_u8(step as u8 * 40, 7, 9);
                game.dm.set_by_name(id, "Color", Value::Color3(colour)).unwrap();
            }
            for (i, &id) in parts.iter().step_by(50).enumerate() {
                game.move_to(id, Vec3::new(i as f32, f32::from(step), 0.0));
            }
            game.dm.destroy(parts[3 + usize::from(step)]).unwrap();
            for _ in 0..300 {
                game.part();
            }
            let stats = frame(&context, &mut scene, &mut game.dm);
            assert_eq!(stats.dispatches, expect_dispatch, "step {step}");
            assert!(stats.ranges > 1);
            assert_gpu_matches(&context, &scene);
        }
        assert!(scene.scene().live(Region::Dynamic) > 0);
        assert!(scene.capacity(Region::Static) > 1024);
    }
}

#[test]
fn renderer_frames_carry_the_upload() {
    let Some(context) = gpu() else { return };
    let mut game = Game::new();
    let ids: Vec<Id> = (0..10).map(|_| game.part()).collect();
    let mut scene = new_scene(&context, &mut game);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = metallic_render::OffscreenTarget::new(&context, 16, 16, format).unwrap();
    let mut renderer =
        metallic_render::Renderer::new(&context, metallic_render::RendererSettings::new(format, 16, 16)).unwrap();
    for (i, &id) in ids.iter().enumerate() {
        game.move_to(id, Vec3::splat(i as f32));
    }
    scene.sync(&mut game.dm);
    let mut stats = UploadStats::default();
    renderer.render_with(target.view(), |encoder| stats = scene.upload(encoder));
    assert_eq!(stats.slots, 10);
    assert_gpu_matches(&context, &scene);
}

/// Every mesh kind's draw list on the GPU equals the scene's: entries on the storage path, copies of the
/// instances on the vertex-buffer path.
fn assert_lists_match(context: &RenderContext, scene: &GpuScene) {
    let bytes = read_back_buffer(context, scene.list_buffer(), scene.list_buffer().size());
    for (kind, segment) in scene.list_segments().iter().enumerate() {
        let entries = scene.scene().draw_list(MeshKind(kind as u16));
        assert_eq!(segment.len as usize, entries.len(), "kind {kind}");
        assert!(segment.len <= segment.capacity);
        let range = |size: usize| segment.first as usize * size..(segment.first + segment.len) as usize * size;
        match scene.binding() {
            InstanceBinding::Storage => {
                let gpu: Vec<u32> = bytemuck::pod_collect_to_vec(&bytes[range(4)]);
                assert_eq!(gpu, entries, "kind {kind}");
            }
            InstanceBinding::Vertex => {
                let gpu: Vec<GpuInstance> = bytemuck::pod_collect_to_vec(&bytes[range(64)]);
                let expected: Vec<GpuInstance> = entries
                    .iter()
                    .map(|&e| *scene.scene().entry_instance(e).expect("a listed slot"))
                    .collect();
                assert_eq!(gpu, expected, "kind {kind}");
            }
        }
    }
}

#[test]
fn the_draw_lists_on_the_gpu_follow_the_scene() {
    for force_vertex_instances in [false, true] {
        let Some(context) = support::gpu_with(&metallic_render::ContextOptions {
            force_vertex_instances,
            ..metallic_render::ContextOptions::default()
        }) else {
            return;
        };
        let mut game = Game::new();
        let shapes = ["Block", "Ball", "Cylinder", "Wedge"];
        let parts: Vec<Id> = (0..600).map(|_| game.part()).collect();
        for (i, &id) in parts.iter().enumerate() {
            let shape = EnumItem::new("PartShape", shapes[i % 4]);
            game.dm.set_by_name(id, "Shape", Value::Enum(shape)).unwrap();
        }
        let mut scene = GpuScene::new(
            &context,
            &mut game.dm,
            Some(game.workspace),
            &RenderComponents::builtin(),
        )
        .unwrap();
        frame(&context, &mut scene, &mut game.dm);
        assert_lists_match(&context, &scene);

        // Every kind outgrows its segment: the lists are laid out again.
        let revision = scene.list_revision();
        for i in 0..1200 {
            let id = game.part();
            let shape = EnumItem::new("PartShape", shapes[i % 4]);
            game.dm.set_by_name(id, "Shape", Value::Enum(shape)).unwrap();
        }
        frame(&context, &mut scene, &mut game.dm);
        assert_ne!(scene.list_revision(), revision);
        assert_lists_match(&context, &scene);

        // Removals, shape changes and recolours, then a few frames of moves (promotions change slots).
        for &id in parts.iter().step_by(7) {
            game.dm.destroy(id).unwrap();
        }
        for &id in parts.iter().skip(1).step_by(14) {
            game.dm
                .set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", "Ball")))
                .unwrap();
            game.dm
                .set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(1, 2, 3)))
                .unwrap();
        }
        let stats = frame(&context, &mut scene, &mut game.dm);
        assert!(stats.list_entries > 0);
        assert_lists_match(&context, &scene);
        for step in 0..PROMOTE_MOVES_UNANCHORED + 1 {
            for &id in parts.iter().skip(2).step_by(14) {
                game.move_to(id, Vec3::new(f32::from(step), 1.0, 0.0));
            }
            frame(&context, &mut scene, &mut game.dm);
        }
        assert!(scene.scene().live(Region::Dynamic) > 0);
        assert_lists_match(&context, &scene);
    }
}
