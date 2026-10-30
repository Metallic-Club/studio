//! Playable debug view of `metallic_physics2d`, drawn with the egui painter.
//!
//! ```text
//! cargo run -p metallic_physics2d --example physics2d_demo
//! cargo run -p metallic_physics2d --example physics2d_demo -- --screenshot shot.png
//! ```
//! Arrows (or A/D) run, Up/Space/Z jump (hold for a higher jump), Down drops through one-way
//! platforms, R resets. `--screenshot <file.png>` plays a short scripted run headless and
//! saves one frame without opening a window.
use eframe::egui::{self, Color32, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2 as EVec2};
use metallic_physics2d::*;
use std::error::Error;

const HERO: BodyId = BodyId(1000);
const GOAL: BodyId = BodyId(40);
const LIFT: BodyId = BodyId(30);
const ELEVATOR: BodyId = BodyId(31);

// Brand palette: graphite, silver/titanium, restrained violet.
const BG: Color32 = Color32::from_rgb(0x1b, 0x1c, 0x20);
const GRID: Color32 = Color32::from_rgb(0x23, 0x25, 0x2a);
const STATIC_FILL: Color32 = Color32::from_rgb(0x34, 0x36, 0x3c);
const STATIC_LINE: Color32 = Color32::from_rgb(0x7d, 0x80, 0x87);
const DYNAMIC_FILL: Color32 = Color32::from_rgb(0x4a, 0x4d, 0x56);
const DYNAMIC_LINE: Color32 = Color32::from_rgb(0xc4, 0xc7, 0xcd);
const SLEEP_LINE: Color32 = Color32::from_rgb(0x6a, 0x6d, 0x74);
const VIOLET: Color32 = Color32::from_rgb(0x9b, 0x82, 0xd4);
const HERO_FILL: Color32 = Color32::from_rgb(0xd7, 0xd5, 0xdc);
const TEXT: Color32 = Color32::from_rgb(0xc9, 0xcb, 0xd0);

fn solid(id: u64, x: f64, y: f64, w: f64, h: f64) -> BodyDesc {
    BodyDesc::new(BodyId(id), BodyKind::Static, Vec2::new(x, y)).with_collider(ColliderDesc::rect(w, h))
}

fn build_scene() -> Scene2d {
    let mut bodies = Vec::new();
    // Ground: a tilemap with a pit and a raised step (4-stud tiles, top at y = 0).
    let columns = 60u32;
    let mut solid_tiles = Vec::new();
    for row in 0..3u32 {
        for col in 0..columns {
            let pit = (22..26).contains(&col);
            let step = row == 2 && (34..40).contains(&col);
            solid_tiles.push(!pit && (row < 2 || step));
        }
    }
    bodies.push(
        BodyDesc::new(BodyId(1), BodyKind::Static, Vec2::new(-80.0, -8.0)).with_collider(
            ColliderDesc::new(Shape2d::Tilemap {
                tile_size: Vec2::new(4.0, 4.0),
                columns,
                solid: solid_tiles,
            }),
        ),
    );
    // Wall-jump shaft on the left.
    bodies.push(solid(2, -82.0, 30.0, 4.0, 60.0));
    bodies.push(solid(3, -64.0, 38.0, 4.0, 44.0));
    // A 30 degree ramp up to a ledge.
    let rise = 24.0 * 30f64.to_radians().tan();
    bodies.push(
        BodyDesc::new(BodyId(4), BodyKind::Static, Vec2::new(-40.0, 0.0)).with_collider(
            ColliderDesc::new(Shape2d::ConvexPolygon {
                points: vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(36.0, 0.0),
                    Vec2::new(36.0, rise),
                    Vec2::new(24.0, rise),
                ],
            }),
        ),
    );
    // One-way platforms.
    for (i, (x, y)) in [(-20.0, 22.0), (-4.0, 30.0), (12.0, 22.0)].into_iter().enumerate() {
        bodies.push(
            BodyDesc::new(BodyId(10 + i as u64), BodyKind::Static, Vec2::new(x, y))
                .with_collider(ColliderDesc::rect(12.0, 1.0).one_way_up()),
        );
    }
    // Moving platform across the pit and an elevator.
    bodies.push(solid(LIFT.0, 8.0, -2.0, 10.0, 1.5));
    bodies.push(solid(ELEVATOR.0, 76.0, 4.0, 8.0, 1.5));
    // Edge-chain hill.
    bodies.push(
        BodyDesc::new(BodyId(20), BodyKind::Static, Vec2::new(80.0, 0.0)).with_collider(
            ColliderDesc::new(Shape2d::EdgeChain {
                points: vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(10.0, 4.0),
                    Vec2::new(20.0, 6.0),
                    Vec2::new(30.0, 4.0),
                    Vec2::new(40.0, 0.0),
                ],
                closed: false,
            }),
        ),
    );
    // Goal sensor on the high platform.
    bodies.push(
        BodyDesc::new(GOAL, BodyKind::Static, Vec2::new(-4.0, 34.0))
            .with_collider(ColliderDesc::rect(6.0, 6.0).sensor()),
    );
    // Crate stack and a bin of balls.
    for i in 0..5 {
        bodies.push(
            BodyDesc::new(BodyId(100 + i), BodyKind::Dynamic, Vec2::new(52.0, 9.5 + 3.0 * i as f64))
                .with_collider(ColliderDesc::rect(3.0, 3.0)),
        );
    }
    bodies.push(solid(50, 104.0, 8.0, 1.0, 8.0));
    bodies.push(solid(51, 124.0, 8.0, 1.0, 8.0));
    for i in 0..36 {
        let x = 106.0 + (i % 9) as f64 * 2.0;
        let y = 14.0 + (i / 9) as f64 * 2.2;
        bodies.push(
            BodyDesc::new(BodyId(200 + i), BodyKind::Dynamic, Vec2::new(x, y)).with_collider(
                ColliderDesc::circle(0.9).with_material(Material2d {
                    density: 1.0,
                    friction: 0.3,
                    restitution: 0.3,
                }),
            ),
        );
    }
    // Pendulum.
    bodies.push(solid(60, 30.0, 40.0, 1.0, 1.0));
    bodies.push(
        BodyDesc::new(BodyId(61), BodyKind::Dynamic, Vec2::new(40.0, 40.0)).with_collider(ColliderDesc::circle(1.8)),
    );
    // Rope bridge over the pit: planks joined by pins between two posts.
    bodies.push(solid(70, -2.0, 1.0, 2.0, 2.0));
    bodies.push(solid(71, 26.0, 1.0, 2.0, 2.0));
    let planks = 7;
    for i in 0..planks {
        bodies.push(
            BodyDesc::new(BodyId(80 + i), BodyKind::Dynamic, Vec2::new(1.0 + 3.5 * i as f64 + 1.75, 1.5))
                .with_collider(ColliderDesc::rect(3.3, 0.6)),
        );
    }
    let mut joints = vec![JointDesc {
        id: JointId(1),
        body_a: BodyId(60),
        body_b: BodyId(61),
        kind: JointKind::Revolute {
            anchor_a: Vec2::ZERO,
            anchor_b: Vec2::new(-10.0, 0.0),
            limits: None,
            motor: None,
        },
        collide_connected: false,
    }];
    let pin = |id: u64, a: BodyId, b: BodyId, anchor_a: Vec2, anchor_b: Vec2| JointDesc {
        id: JointId(id),
        body_a: a,
        body_b: b,
        kind: JointKind::Revolute {
            anchor_a,
            anchor_b,
            limits: None,
            motor: None,
        },
        collide_connected: false,
    };
    joints.push(pin(10, BodyId(70), BodyId(80), Vec2::new(1.0, 0.5), Vec2::new(-1.75, 0.0)));
    for i in 0..planks - 1 {
        joints.push(pin(
            11 + i,
            BodyId(80 + i),
            BodyId(81 + i),
            Vec2::new(1.75, 0.0),
            Vec2::new(-1.75, 0.0),
        ));
    }
    joints.push(pin(
        30,
        BodyId(80 + planks - 1),
        BodyId(71),
        Vec2::new(1.75, 0.0),
        Vec2::new(-1.0, 0.5),
    ));
    Scene2d { bodies, joints }
}

/// The demo state: physics world, scene and camera.
struct Demo {
    world: PhysicsWorld2d,
    scene: Scene2d,
    time: f64,
    camera: Vec2,
    in_goal: bool,
    last_event: Option<CharacterEventKind>,
    step_ms: f64,
}

impl Demo {
    fn new() -> Result<Demo, Physics2dError> {
        let scene = build_scene();
        let mut world = PhysicsWorld2d::new(WorldConfig::default())?;
        world.sync_from(&scene)?;
        let start = Vec2::new(-56.0, 4.0);
        world.add_character(HERO, CharacterConfig::default(), start)?;
        Ok(Demo {
            world,
            scene,
            time: 0.0,
            camera: start,
            in_goal: false,
            last_event: None,
            step_ms: 0.0,
        })
    }

    fn update(&mut self, input: CharacterInput, dt: f64) -> Result<(), Physics2dError> {
        self.time += dt;
        let t = self.time;
        if let Some(lift) = self.scene.body_mut(LIFT) {
            lift.position.x = 9.0 + 8.0 * (t * 0.8).sin();
        }
        if let Some(e) = self.scene.body_mut(ELEVATOR) {
            e.position.y = 12.0 - 8.0 * (t * 0.6).cos();
        }
        self.world.set_character_input(HERO, input)?;
        let started = std::time::Instant::now();
        self.world.step(&mut self.scene, dt)?;
        self.step_ms = started.elapsed().as_secs_f64() * 1000.0;
        for e in self.world.drain_sensor_events() {
            if e.sensor == GOAL && e.other == HERO {
                self.in_goal = e.kind == SensorEventKind::Enter;
            }
        }
        if let Some(e) = self.world.drain_character_events().last() {
            self.last_event = Some(e.kind);
        }
        let _ = self.world.drain_touched();
        if let Some(c) = self.world.character(HERO) {
            if c.position.y < -60.0 {
                self.world.teleport_character(HERO, Vec2::new(-56.0, 4.0))?;
            }
            // Smooth camera follow.
            let k = 1.0 - (-dt * 6.0).exp();
            self.camera = self.camera.lerp(c.position + Vec2::new(0.0, 6.0), k);
        }
        Ok(())
    }

    fn paint(&self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, BG);
        let scale = (rect.height() / 84.0).max(1.0);
        let center = rect.center();
        let cam = self.camera;
        let to_screen = |p: Vec2| -> Pos2 {
            Pos2::new(
                center.x + ((p.x - cam.x) as f32) * scale,
                center.y - ((p.y - cam.y) as f32) * scale,
            )
        };

        // Grid every 4 studs.
        let half_w = f64::from(rect.width() / scale) * 0.5;
        let half_h = f64::from(rect.height() / scale) * 0.5;
        let mut gx = ((cam.x - half_w) / 4.0).floor() * 4.0;
        while gx < cam.x + half_w {
            let a = to_screen(Vec2::new(gx, cam.y - half_h));
            let b = to_screen(Vec2::new(gx, cam.y + half_h));
            painter.line_segment([a, b], Stroke::new(1.0, GRID));
            gx += 4.0;
        }
        let mut gy = ((cam.y - half_h) / 4.0).floor() * 4.0;
        while gy < cam.y + half_h {
            let a = to_screen(Vec2::new(cam.x - half_w, gy));
            let b = to_screen(Vec2::new(cam.x + half_w, gy));
            painter.line_segment([a, b], Stroke::new(1.0, GRID));
            gy += 4.0;
        }

        for s in self.world.debug_shapes() {
            let (fill, line) = match s.kind {
                _ if s.sensor => (
                    Color32::from_rgba_unmultiplied(0x9b, 0x82, 0xd4, if self.in_goal { 90 } else { 30 }),
                    VIOLET,
                ),
                DebugBodyKind::Character => (HERO_FILL, VIOLET),
                DebugBodyKind::Kinematic => (STATIC_FILL, VIOLET),
                DebugBodyKind::Dynamic if s.sleeping => (DYNAMIC_FILL, SLEEP_LINE),
                DebugBodyKind::Dynamic => (DYNAMIC_FILL, DYNAMIC_LINE),
                DebugBodyKind::Static if s.one_way => (STATIC_FILL, VIOLET),
                DebugBodyKind::Static => (STATIC_FILL, STATIC_LINE),
            };
            let stroke = Stroke::new(1.5, line);
            match &s.geometry {
                DebugGeometry::Polygon { points } => {
                    let pts: Vec<Pos2> = points.iter().map(|p| to_screen(*p)).collect();
                    painter.add(Shape::convex_polygon(pts.clone(), fill, stroke));
                    if s.one_way && points.len() == 4 {
                        // Highlight the solid top edge.
                        painter.line_segment([pts[2], pts[3]], Stroke::new(3.0, VIOLET));
                    }
                }
                DebugGeometry::Circle {
                    center,
                    radius,
                    angle,
                } => {
                    let c = to_screen(*center);
                    let r = *radius as f32 * scale;
                    painter.circle(c, r, fill, stroke);
                    let spoke = *center + Vec2::new(angle.cos(), angle.sin()) * *radius;
                    painter.line_segment([c, to_screen(spoke)], stroke);
                }
                DebugGeometry::Capsule { a, b, radius } => {
                    let r = *radius as f32 * scale;
                    painter.line_segment([to_screen(*a), to_screen(*b)], Stroke::new(2.0 * r, fill));
                    painter.circle(to_screen(*a), r, fill, stroke);
                    painter.circle(to_screen(*b), r, fill, stroke);
                }
                DebugGeometry::Polyline { points, closed } => {
                    let mut pts: Vec<Pos2> = points.iter().map(|p| to_screen(*p)).collect();
                    if *closed && let Some(first) = pts.first().copied() {
                        pts.push(first);
                    }
                    painter.add(Shape::line(pts, Stroke::new(2.5, STATIC_LINE)));
                }
            }
        }

        // Joints: a thin line between the two anchors.
        for j in &self.scene.joints {
            let anchors = match &j.kind {
                JointKind::Revolute { anchor_a, anchor_b, .. }
                | JointKind::Prismatic { anchor_a, anchor_b, .. }
                | JointKind::Distance { anchor_a, anchor_b, .. }
                | JointKind::Weld { anchor_a, anchor_b } => (*anchor_a, *anchor_b),
            };
            let (Some(a), Some(b)) = (self.scene.body(j.body_a), self.scene.body(j.body_b)) else {
                continue;
            };
            let pa = Pose2::new(a.position, a.angle).transform_point(anchors.0);
            let pb = Pose2::new(b.position, b.angle).transform_point(anchors.1);
            painter.line_segment([to_screen(pa), to_screen(b.position)], Stroke::new(1.0, VIOLET));
            painter.circle_filled(to_screen(pa), 2.5, VIOLET);
            painter.circle_filled(to_screen(pb), 2.5, VIOLET);
        }

        // Character facing marker.
        if let Some(c) = self.world.character(HERO) {
            let eye = c.position + Vec2::new(0.6 * c.facing, 1.2);
            painter.circle_filled(to_screen(eye), 0.35 * scale, BG);
            self.hud(&painter, rect, &c);
        }
    }

    fn hud(&self, painter: &egui::Painter, rect: Rect, c: &CharacterState) {
        let panel = Rect::from_min_size(rect.min + EVec2::new(12.0, 12.0), EVec2::new(430.0, 128.0));
        painter.rect(
            panel,
            6.0,
            Color32::from_rgba_unmultiplied(0x14, 0x15, 0x18, 220),
            Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3c, 0x42)),
            StrokeKind::Inside,
        );
        let flag = |b: bool| if b { "yes" } else { "no" };
        let wall = match (c.wall, c.wall_sliding) {
            (Some(WallSide::Left), true) => "left, sliding",
            (Some(WallSide::Right), true) => "right, sliding",
            (Some(WallSide::Left), false) => "left",
            (Some(WallSide::Right), false) => "right",
            (None, _) => "none",
        };
        let lines = [
            "Metallic 2D physics: arrows run, Up/Space/Z jump, Down drops, R resets".to_string(),
            format!(
                "grounded {}  one-way {}  wall {}",
                flag(c.grounded),
                flag(c.on_one_way),
                wall
            ),
            format!(
                "velocity ({:6.1}, {:6.1})  coyote {:.2}s  buffer {:.2}s",
                c.velocity.x, c.velocity.y, c.coyote_remaining, c.jump_buffer_remaining
            ),
            format!(
                "last event {}  goal {}",
                self.last_event.map_or("-".to_string(), |e| format!("{e:?}")),
                if self.in_goal { "reached" } else { "-" }
            ),
            format!(
                "{} bodies, {} joints, step {:.2} ms",
                self.scene.bodies.len(),
                self.scene.joints.len(),
                self.step_ms
            ),
        ];
        for (i, line) in lines.iter().enumerate() {
            painter.text(
                panel.min + EVec2::new(12.0, 10.0 + 22.0 * i as f32),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::monospace(13.0),
                if i == 0 { HERO_FILL } else { TEXT },
            );
        }
    }
}

fn keyboard_input(ctx: &egui::Context) -> (CharacterInput, bool) {
    ctx.input(|i| {
        let left = i.key_down(Key::ArrowLeft) || i.key_down(Key::A);
        let right = i.key_down(Key::ArrowRight) || i.key_down(Key::D);
        let input = CharacterInput {
            move_x: f64::from(u8::from(right)) - f64::from(u8::from(left)),
            jump: i.key_down(Key::ArrowUp) || i.key_down(Key::Space) || i.key_down(Key::Z),
            drop_through: i.key_down(Key::ArrowDown) || i.key_down(Key::S),
        };
        (input, i.key_pressed(Key::R))
    })
}

struct App {
    demo: Demo,
    error: Option<String>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (input, reset) = keyboard_input(&ctx);
        if reset && let Ok(d) = Demo::new() {
            self.demo = d;
        }
        let dt = f64::from(ctx.input(|i| i.stable_dt)).clamp(0.0, 0.1);
        if let Err(e) = self.demo.update(input, dt) {
            self.error = Some(e.to_string());
        }
        self.demo.paint(ui);
        if let Some(e) = &self.error {
            ui.painter().text(
                ui.max_rect().center(),
                egui::Align2::CENTER_CENTER,
                e,
                egui::FontId::proportional(16.0),
                Color32::WHITE,
            );
        }
        ctx.request_repaint();
    }
}

/// Plays a short scripted run (run right, jump onto the ramp, hop the platforms) and saves
/// one rendered frame, without opening a window.
fn screenshot(path: &str) -> Result<(), Box<dyn Error>> {
    let mut demo = Demo::new()?;
    let dt = 1.0 / 60.0;
    for f in 0..150 {
        let t = f as f64 * dt;
        let input = CharacterInput {
            move_x: 1.0,
            jump: (0.95..1.35).contains(&t),
            drop_through: false,
        };
        demo.update(input, dt)?;
    }
    let mut harness = egui_kittest::Harness::builder()
        .with_size(EVec2::new(1280.0, 720.0))
        .wgpu()
        .build_ui_state(|ui, demo: &mut Demo| demo.paint(ui), demo);
    harness.run_steps(2);
    let image = harness.render()?;
    image.save(path)?;
    println!("saved {path}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--screenshot") {
        let path = args.get(i + 1).map_or("physics2d_demo.png", String::as_str);
        return screenshot(path);
    }
    let demo = Demo::new()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metallic 2D physics demo")
            .with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Metallic 2D physics demo",
        options,
        Box::new(|_cc| Ok(Box::new(App { demo, error: None }))),
    )?;
    Ok(())
}
