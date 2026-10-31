//! Physics debug draw of a running game (#23): Parkour shows a wireframe per collider, the
//! character's 15 touch volumes, and its contact with the ground; off draws nothing.
use metallic_render::DebugShape;
use metallic_render::meshes::MeshKind;
use metallic_runtime::{DebugDrawOptions, FrameInput, Game, StdoutSink, TICK};
use std::path::PathBuf;
use std::rc::Rc;

fn parkour() -> Game {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let mut game = Game::open(&dir, None, Rc::new(StdoutSink)).expect("load Parkour");
    game.start().expect("start");
    for _ in 0..30 {
        game.frame(&FrameInput::default(), TICK);
    }
    game
}

#[test]
fn parkour_draws_colliders_contacts_and_touch_volumes() {
    let mut game = parkour();
    let mut shapes: Vec<DebugShape> = Vec::new();
    game.debug_shapes(&mut shapes);
    assert!(shapes.is_empty(), "off by default");

    game.set_debug_draw(DebugDrawOptions {
        shapes: true,
        ..DebugDrawOptions::default()
    });
    game.debug_shapes(&mut shapes);
    let colliders = shapes.len();
    assert!(
        colliders > 20,
        "one wireframe per collider, got {colliders}"
    );

    game.set_debug_draw(DebugDrawOptions {
        touch_volumes: true,
        ..DebugDrawOptions::default()
    });
    game.debug_shapes(&mut shapes);
    // A capsule is a cylinder and two spheres; 15 body parts.
    let cylinders = shapes
        .iter()
        .filter(|s| s.kind == MeshKind::Cylinder)
        .count();
    assert_eq!(cylinders, 15);

    game.set_debug_draw(DebugDrawOptions::ALL);
    game.debug_shapes(&mut shapes);
    assert!(shapes.len() > colliders + 45, "contacts are drawn too");

    game.set_debug_draw(DebugDrawOptions::default());
    game.debug_shapes(&mut shapes);
    assert!(shapes.is_empty());
}
