//! Toolbar and Explorer icons, drawn with egui shapes (no icon font, so nothing depends on which
//! glyphs a bundled font happens to contain). Silver on graphite, 16 px.
//!
//! [`Icon`] is a toolbar or command icon; [`ClassIcon`] is the icon of an instance class, chosen
//! by [`class_icon`] (exact class first, then by superclass) and drawn by [`paint_class_icon`].
//! Strokes: 1.6 px for toolbar icons, 1.3 px for class icons, 1.0 px for fine detail.
use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use metallic_core::classes::is_subclass;

/// A toolbar icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Arrow pointer.
    Select,
    /// Four-way arrows.
    Move,
    /// Circular arrow.
    Rotate,
    /// Diagonal double arrow with a box.
    Scale,
    /// A bounding box with corner handles (move, rotate and scale in one).
    Transform,
    /// A 3 x 3 grid.
    Snap,
    /// A globe (world/local axes).
    Space,
    /// Arrow turning left.
    Undo,
    /// Arrow turning right.
    Redo,
    /// Triangle.
    Play,
    /// Square.
    Stop,
    /// Two figures (Team Test and other multi-player modes).
    Players,
    /// A paint brush.
    Brush,
    /// A ruler.
    Measure,
    /// Two bars against a left edge.
    Align,
    /// Two bars evenly spaced between two edges.
    Distribute,
    /// A triangle and its reflection across a dashed axis.
    Mirror,
    /// Two filled triangles across a dashed axis.
    MirrorCopy,
    /// One filled square and two outlined copies in a row.
    Array,
    /// A square and its dashed twin.
    SelectSimilar,
    /// A dashed rectangle.
    SelectBox,
    /// A dashed loop.
    SelectLasso,
    /// A brush circle with its centre.
    SelectPaint,
    /// A game controller (input actions).
    Gamepad,
    /// A box with its centre dot (pivot or selection centre).
    PivotMode,
    /// A crosshair (the pivot point).
    Pivot,
    /// Arrow down onto a surface.
    Drop,
    /// Two overlapping squares.
    Duplicate,
    /// A bin.
    Delete,
    /// Corner brackets around a square (a new Model).
    Group,
    /// A pencil.
    Rename,
    /// A dashed square around a filled one.
    SelectAll,
    /// A disk.
    Save,
    /// A cross.
    Close,
    /// A dashed square struck through.
    Deselect,
    /// A gear.
    Settings,
    /// Viewfinder corners around a dot.
    FrameSelection,
    /// Viewfinder corners around a square.
    FrameAll,
    /// A magnifier.
    Search,
    /// A prompt in a window.
    Commands,
    /// A plus in a circle.
    Insert,
    /// A drop of paint.
    Colour,
    /// A bar with its first item highlighted (the contextual toolbar).
    ContextBar,
    /// A key (key bindings).
    Keyboard,
    /// A half-filled circle (the look of a part: the style picker).
    Style,
    /// A double chevron (more toolbar groups). Keep it last: [`Icon::ALL`] ends with it.
    More,
}

impl Icon {
    /// Every icon, in declaration order (tests paint each one and check the order).
    pub const ALL: [Icon; 46] = [
        Icon::Select,
        Icon::Move,
        Icon::Rotate,
        Icon::Scale,
        Icon::Transform,
        Icon::Snap,
        Icon::Space,
        Icon::Undo,
        Icon::Redo,
        Icon::Play,
        Icon::Stop,
        Icon::Players,
        Icon::Brush,
        Icon::Measure,
        Icon::Align,
        Icon::Distribute,
        Icon::Mirror,
        Icon::MirrorCopy,
        Icon::Array,
        Icon::SelectSimilar,
        Icon::SelectBox,
        Icon::SelectLasso,
        Icon::SelectPaint,
        Icon::Gamepad,
        Icon::PivotMode,
        Icon::Pivot,
        Icon::Drop,
        Icon::Duplicate,
        Icon::Delete,
        Icon::Group,
        Icon::Rename,
        Icon::SelectAll,
        Icon::Save,
        Icon::Close,
        Icon::Deselect,
        Icon::Settings,
        Icon::FrameSelection,
        Icon::FrameAll,
        Icon::Search,
        Icon::Commands,
        Icon::Insert,
        Icon::Colour,
        Icon::ContextBar,
        Icon::Keyboard,
        Icon::Style,
        Icon::More,
    ];
}

fn arc(centre: Pos2, r: f32, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    ellipse_arc(centre, vec2(r, r), from, to, n)
}

fn ellipse_arc(centre: Pos2, radius: Vec2, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            centre + vec2(a.cos() * radius.x, a.sin() * radius.y)
        })
        .collect()
}

fn head(painter: &egui::Painter, tip: Pos2, dir: Vec2, c: Color32) {
    head_sized(painter, tip + dir.normalized() * 1.5, dir, 5.0, 3.0, c);
}

/// A filled arrow head with its point at `tip`, `len` long and `half` wide on each side.
fn head_sized(painter: &egui::Painter, tip: Pos2, dir: Vec2, len: f32, half: f32, c: Color32) {
    let d = dir.normalized();
    let side = vec2(-d.y, d.x);
    painter.add(Shape::convex_polygon(
        vec![
            tip,
            tip - d * len + side * half,
            tip - d * len - side * half,
        ],
        c,
        Stroke::NONE,
    ));
}

fn poly(painter: &egui::Painter, o: Pos2, pts: &[(f32, f32)], closed: bool, s: Stroke) {
    let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| o + vec2(x, y)).collect();
    painter.add(if closed {
        Shape::closed_line(pts, s)
    } else {
        Shape::line(pts, s)
    });
}

fn filled(painter: &egui::Painter, o: Pos2, pts: &[(f32, f32)], c: Color32) {
    let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| o + vec2(x, y)).collect();
    painter.add(Shape::convex_polygon(pts, c, Stroke::NONE));
}

fn line(painter: &egui::Painter, o: Pos2, a: (f32, f32), b: (f32, f32), s: Stroke) {
    painter.line_segment([o + vec2(a.0, a.1), o + vec2(b.0, b.1)], s);
}

fn outline(painter: &egui::Painter, r: Rect, rounding: f32, s: Stroke) {
    painter.rect_stroke(r, rounding, s, egui::StrokeKind::Middle);
}

fn dashed(painter: &egui::Painter, pts: &[Pos2], c: Color32) {
    painter.extend(Shape::dashed_line(pts, Stroke::new(1.2, c), 2.0, 1.6));
}

fn dashed_rect(painter: &egui::Painter, r: Rect, c: Color32) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    dashed(painter, &pts, c);
}

/// Viewfinder corners of a `size` square around `o`.
fn corners(painter: &egui::Painter, o: Pos2, size: f32, len: f32, s: Stroke) {
    let b = Rect::from_center_size(o, Vec2::splat(size));
    for (p, d) in [
        (b.left_top(), vec2(1.0, 1.0)),
        (b.right_top(), vec2(-1.0, 1.0)),
        (b.left_bottom(), vec2(1.0, -1.0)),
        (b.right_bottom(), vec2(-1.0, -1.0)),
    ] {
        painter.line_segment([p, p + vec2(d.x * len, 0.0)], s);
        painter.line_segment([p, p + vec2(0.0, d.y * len)], s);
    }
}

/// Paints `icon` centred in `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, c: Color32) {
    let s = Stroke::new(1.6, c);
    let thin = Stroke::new(1.3, c);
    let fine = Stroke::new(1.0, c);
    let o = rect.center();
    let r = 6.5;
    let sq = |x: f32, y: f32, size: f32| Rect::from_center_size(o + vec2(x, y), Vec2::splat(size));
    match icon {
        Icon::Select => {
            let pts = vec![
                o + vec2(-4.0, -7.0),
                o + vec2(5.0, 1.5),
                o + vec2(0.5, 2.0),
                o + vec2(-1.5, 7.0),
                o + vec2(-3.5, 6.0),
                o + vec2(-1.5, 1.5),
                o + vec2(-4.0, 3.5),
            ];
            painter.add(Shape::closed_line(pts, s));
        }
        Icon::Move => {
            for d in [
                vec2(1.0, 0.0),
                vec2(-1.0, 0.0),
                vec2(0.0, 1.0),
                vec2(0.0, -1.0),
            ] {
                painter.line_segment([o, o + d * (r - 1.0)], s);
                head(painter, o + d * (r + 0.5), d, c);
            }
        }
        Icon::Rotate => {
            let a0 = -0.6f32;
            let a1 = PI * 1.45;
            painter.add(Shape::line(arc(o, r - 1.0, a0, a1, 18), s));
            let end = o + vec2(a0.cos(), a0.sin()) * (r - 1.0);
            head(painter, end, vec2(a0.sin(), -a0.cos()) * -1.0, c);
        }
        Icon::Scale => {
            outline(painter, sq(-2.0, 2.0, 7.0), 0.0, s);
            painter.line_segment([o + vec2(-1.0, 1.0), o + vec2(5.0, -5.0)], s);
            head(painter, o + vec2(6.0, -6.0), vec2(1.0, -1.0), c);
        }
        Icon::Transform => {
            // A bounding box with its corner handles.
            outline(painter, sq(0.0, 0.0, 11.0), 0.0, thin);
            for (x, y) in [(-5.5, -5.5), (5.5, -5.5), (-5.5, 5.5), (5.5, 5.5)] {
                painter.rect_filled(sq(x, y, 3.6), 0.5, c);
            }
            painter.circle_filled(o, 1.3, c);
        }
        Icon::Snap => {
            for i in 0..4 {
                let t = -6.0 + i as f32 * 4.0;
                line(painter, o, (t, -6.0), (t, 6.0), fine);
                line(painter, o, (-6.0, t), (6.0, t), fine);
            }
        }
        Icon::Space => {
            painter.circle_stroke(o, r, s);
            line(painter, o, (-r, 0.0), (r, 0.0), fine);
            painter.add(Shape::closed_line(
                ellipse_arc(o, vec2(3.0, r), 0.0, TAU, 24),
                fine,
            ));
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let centre = o + vec2(0.0, 1.5);
            let pts: Vec<Pos2> = arc(centre, 5.0, PI, TAU + 0.6, 16)
                .into_iter()
                .map(|p| pos2(o.x + (p.x - o.x) * flip, p.y))
                .collect();
            let start = pts[0];
            painter.add(Shape::line(pts, s));
            head(painter, start + vec2(0.0, 1.5), vec2(0.0, 1.0), c);
        }
        Icon::Play => filled(painter, o, &[(-4.0, -6.0), (6.0, 0.0), (-4.0, 6.0)], c),
        Icon::Stop => {
            painter.rect_filled(sq(0.0, 0.0, 10.0), 1.5, c);
        }
        Icon::Players => paint_class_icon(painter, rect, ClassIcon::Players, c),
        Icon::Brush => {
            // Handle from the top right, bristles towards the bottom left.
            let u = vec2(-1.0, 1.0).normalized();
            let v = vec2(1.0, 1.0).normalized();
            let b = o + vec2(1.0, -1.0);
            painter.line_segment([b, o + vec2(6.5, -6.5)], Stroke::new(2.0, c));
            painter.add(Shape::convex_polygon(
                vec![
                    b + v * 2.6,
                    b + v * 2.6 + u * 3.0,
                    b + u * 7.5,
                    b - v * 2.6 + u * 3.0,
                    b - v * 2.6,
                ],
                c,
                Stroke::NONE,
            ));
        }
        Icon::Measure => {
            let body = Rect::from_min_max(o + vec2(-7.0, -3.0), o + vec2(7.0, 3.5));
            outline(painter, body, 1.0, thin);
            for (i, x) in [-4.0f32, -1.5, 1.0, 3.5].into_iter().enumerate() {
                let len = if i % 2 == 0 { 3.2 } else { 1.8 };
                line(painter, o, (x, -3.0), (x, -3.0 + len), fine);
            }
        }
        Icon::Align => {
            line(painter, o, (-6.5, -7.0), (-6.5, 7.0), s);
            painter.rect_filled(
                Rect::from_min_max(o + vec2(-4.0, -5.0), o + vec2(6.0, -1.5)),
                0.5,
                c,
            );
            painter.rect_filled(
                Rect::from_min_max(o + vec2(-4.0, 1.5), o + vec2(2.0, 5.0)),
                0.5,
                c,
            );
        }
        Icon::Distribute => {
            line(painter, o, (-6.5, -7.0), (-6.5, 7.0), thin);
            line(painter, o, (6.5, -7.0), (6.5, 7.0), thin);
            for x in [-2.6, 2.6] {
                painter.rect_filled(sq(x, 0.0, 3.0).expand2(vec2(0.0, 2.5)), 0.5, c);
            }
        }
        Icon::Mirror | Icon::MirrorCopy => {
            dashed(painter, &[o + vec2(0.0, -7.5), o + vec2(0.0, 7.5)], c);
            let left = [(-2.0, -5.0), (-2.0, 5.0), (-7.0, 5.0)];
            if icon == Icon::MirrorCopy {
                filled(painter, o, &left, c);
            } else {
                poly(painter, o, &left, true, thin);
            }
            filled(painter, o, &[(2.0, -5.0), (7.0, 5.0), (2.0, 5.0)], c);
        }
        Icon::Array => {
            painter.rect_filled(sq(-5.0, 0.0, 4.0), 0.5, c);
            outline(painter, sq(0.0, 0.0, 4.0), 0.5, thin);
            outline(painter, sq(5.0, 0.0, 4.0), 0.5, thin);
        }
        Icon::SelectSimilar => {
            painter.rect_filled(sq(-2.5, -2.5, 7.0), 1.0, c);
            dashed_rect(painter, sq(3.0, 3.0, 7.0), c);
        }
        Icon::SelectBox => dashed_rect(painter, sq(0.0, 0.0, 12.0).shrink2(vec2(0.0, 1.5)), c),
        Icon::SelectLasso => {
            let mut loop_pts = ellipse_arc(o + vec2(0.5, -1.5), vec2(6.0, 4.0), 0.0, TAU, 24);
            loop_pts.push(loop_pts[0]);
            dashed(painter, &loop_pts, c);
            poly(
                painter,
                o,
                &[(-3.5, 1.5), (-4.5, 4.5), (-2.5, 6.5)],
                false,
                thin,
            );
        }
        Icon::SelectPaint => {
            painter.circle_stroke(o, 5.5, thin);
            painter.circle_filled(o, 1.6, c);
        }
        Icon::Gamepad => paint_class_icon(painter, rect, ClassIcon::InputContext, c),
        Icon::PivotMode => {
            outline(painter, sq(0.0, 0.0, 12.0), 1.0, thin);
            painter.circle_filled(o, 2.0, c);
        }
        Icon::Pivot => {
            painter.circle_stroke(o, 3.6, thin);
            painter.circle_filled(o, 1.1, c);
            for (a, b) in [
                ((0.0, -7.0), (0.0, -4.5)),
                ((0.0, 4.5), (0.0, 7.0)),
                ((-7.0, 0.0), (-4.5, 0.0)),
                ((4.5, 0.0), (7.0, 0.0)),
            ] {
                line(painter, o, a, b, thin);
            }
        }
        Icon::Drop => {
            line(painter, o, (0.0, -7.0), (0.0, 1.0), s);
            head_sized(painter, o + vec2(0.0, 4.0), vec2(0.0, 1.0), 4.0, 3.0, c);
            line(painter, o, (-6.5, 6.5), (6.5, 6.5), s);
        }
        Icon::Duplicate => {
            let back = [
                (-2.0, 2.0),
                (-6.0, 2.0),
                (-6.0, -6.0),
                (2.0, -6.0),
                (2.0, -2.0),
            ];
            poly(painter, o, &back, false, thin);
            outline(
                painter,
                Rect::from_min_max(o + vec2(-2.0, -2.0), o + vec2(6.0, 6.0)),
                1.0,
                thin,
            );
        }
        Icon::Delete => {
            line(painter, o, (-6.5, -4.0), (6.5, -4.0), thin);
            poly(
                painter,
                o,
                &[(-2.0, -4.0), (-2.0, -6.5), (2.0, -6.5), (2.0, -4.0)],
                false,
                thin,
            );
            poly(
                painter,
                o,
                &[(-4.8, -4.0), (-4.0, 6.5), (4.0, 6.5), (4.8, -4.0)],
                false,
                thin,
            );
            line(painter, o, (-1.3, -1.5), (-1.3, 4.0), fine);
            line(painter, o, (1.3, -1.5), (1.3, 4.0), fine);
        }
        Icon::Group => paint_class_icon(painter, rect, ClassIcon::Model, c),
        Icon::Rename => {
            let u = vec2(1.0, -1.0).normalized();
            let v = vec2(1.0, 1.0).normalized();
            let tip = o + vec2(-6.0, 6.0);
            let (a, b, w) = (3.5, 13.5, 2.3);
            painter.add(Shape::closed_line(
                vec![
                    tip,
                    tip + u * a + v * w,
                    tip + u * b + v * w,
                    tip + u * b - v * w,
                    tip + u * a - v * w,
                ],
                thin,
            ));
            painter.line_segment([tip + u * a + v * w, tip + u * a - v * w], thin);
            painter.line_segment([tip + u * 10.5 + v * w, tip + u * 10.5 - v * w], fine);
        }
        Icon::SelectAll => {
            dashed_rect(painter, sq(0.0, 0.0, 13.0), c);
            painter.rect_filled(sq(0.0, 0.0, 5.0), 0.5, c);
        }
        Icon::Save => {
            let disk = [
                (-6.5, -6.5),
                (4.0, -6.5),
                (6.5, -4.0),
                (6.5, 6.5),
                (-6.5, 6.5),
            ];
            poly(painter, o, &disk, true, thin);
            outline(
                painter,
                Rect::from_min_max(o + vec2(-3.5, -6.5), o + vec2(2.5, -2.5)),
                0.0,
                fine,
            );
            painter.rect_filled(
                Rect::from_min_max(o + vec2(-4.0, 1.0), o + vec2(4.0, 6.5)),
                0.5,
                c,
            );
        }
        Icon::Close => {
            line(painter, o, (-5.0, -5.0), (5.0, 5.0), s);
            line(painter, o, (-5.0, 5.0), (5.0, -5.0), s);
        }
        Icon::Deselect => {
            dashed_rect(painter, sq(0.0, 0.0, 12.0), c);
            line(painter, o, (-7.0, 7.0), (7.0, -7.0), thin);
        }
        Icon::Settings => {
            for k in 0..6 {
                let a = k as f32 * TAU / 6.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([o + d * 4.5, o + d * 7.0], Stroke::new(2.6, c));
            }
            painter.circle_stroke(o, 4.4, s);
            painter.circle_stroke(o, 1.7, thin);
        }
        Icon::FrameSelection | Icon::FrameAll => {
            corners(painter, o, 14.0, 3.5, thin);
            if icon == Icon::FrameSelection {
                painter.circle_filled(o, 2.2, c);
            } else {
                outline(painter, sq(0.0, 0.0, 6.0), 0.5, thin);
            }
        }
        Icon::Search => {
            painter.circle_stroke(o + vec2(-1.5, -1.5), 4.6, s);
            line(painter, o, (2.0, 2.0), (6.5, 6.5), Stroke::new(2.2, c));
        }
        Icon::Commands => {
            outline(
                painter,
                sq(0.0, 0.0, 14.0).shrink2(vec2(0.0, 1.5)),
                1.5,
                thin,
            );
            poly(
                painter,
                o,
                &[(-4.0, -2.5), (-1.5, 0.0), (-4.0, 2.5)],
                false,
                thin,
            );
            line(painter, o, (0.5, 2.5), (4.0, 2.5), thin);
        }
        Icon::Insert => {
            painter.circle_stroke(o, 6.3, thin);
            line(painter, o, (-3.0, 0.0), (3.0, 0.0), s);
            line(painter, o, (0.0, -3.0), (0.0, 3.0), s);
        }
        Icon::Colour => {
            // A drop: the arc between the tangents from the tip, then the tip.
            let centre = o + vec2(0.0, 2.0);
            let mut pts = arc(centre, 4.8, -0.53, PI + 0.53, 16);
            pts.push(o + vec2(0.0, -7.0));
            painter.add(Shape::convex_polygon(pts, c, Stroke::NONE));
        }
        Icon::ContextBar => {
            outline(
                painter,
                sq(0.0, 0.0, 14.0).shrink2(vec2(0.0, 3.5)),
                1.5,
                thin,
            );
            painter.rect_filled(sq(-3.5, 0.0, 3.0), 0.5, c);
            painter.circle_filled(o + vec2(1.0, 0.0), 0.9, c);
            painter.circle_filled(o + vec2(4.0, 0.0), 0.9, c);
        }
        Icon::Keyboard => paint_class_icon(painter, rect, ClassIcon::InputBinding, c),
        Icon::Style => paint_class_icon(painter, rect, ClassIcon::ColorCorrection, c),
        Icon::More => {
            poly(
                painter,
                o,
                &[(-5.0, -4.0), (-1.0, 0.0), (-5.0, 4.0)],
                false,
                s,
            );
            poly(painter, o, &[(1.0, -4.0), (5.0, 0.0), (1.0, 4.0)], false, s);
        }
    }
}

/// The icon of an instance class (see [`class_icon`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassIcon {
    /// Workspace: a globe.
    Workspace,
    /// Folder.
    Folder,
    /// Model: corner brackets around a square.
    Model,
    /// A block-shaped part: a cube.
    Block,
    /// A ball-shaped part: a sphere.
    Ball,
    /// A cylinder-shaped part.
    Cylinder,
    /// A wedge-shaped part.
    Wedge,
    /// SpawnLocation: a pad with an arrow up.
    Spawn,
    /// Checkpoint: a flag.
    Checkpoint,
    /// Script: a page of lines.
    Script,
    /// LocalScript: a page with a round badge (runs on the client).
    LocalScript,
    /// ModuleScript: a page with a diamond badge (a library).
    ModuleScript,
    /// RemoteEvent: arrows both ways.
    RemoteEvent,
    /// UnreliableRemoteEvent: dashed arrows both ways.
    UnreliableRemoteEvent,
    /// RemoteFunction: a call and its answer.
    RemoteFunction,
    /// Humanoid: a standing figure.
    Humanoid,
    /// Player and StarterPlayer: one figure.
    Player,
    /// Players: two figures.
    Players,
    /// ScreenGui, StarterGui, PlayerGui: a window.
    Gui,
    /// Frame: a plain rectangle.
    Frame,
    /// TextLabel: a rectangle with lines of text.
    TextLabel,
    /// TextButton and other buttons: a rounded button.
    Button,
    /// ImageButton: a picture.
    ImageButton,
    /// Lighting: a light bulb.
    Lighting,
    /// Sky: the sun on the horizon.
    Sky,
    /// Camera: a body with a lens at the front.
    Camera,
    /// Atmosphere: haze lines.
    Atmosphere,
    /// Clouds: a cloud.
    Clouds,
    /// BloomEffect: a bright core in a ring.
    Bloom,
    /// ColorCorrectionEffect: a half-filled circle.
    ColorCorrection,
    /// ColorGradingEffect: three sliders.
    ColorGrading,
    /// ReplicatedStorage and ServerStorage: a box.
    Storage,
    /// ServerScriptService: a server.
    Server,
    /// StarterPlayerScripts and PlayerScripts: a folder of scripts.
    ScriptFolder,
    /// RunService: a stopwatch.
    RunService,
    /// InputContext: a game controller.
    InputContext,
    /// InputAction: a bolt (something fires).
    InputAction,
    /// InputBinding: a key.
    InputBinding,
    /// Anything else: a rounded square. Keep it last: [`ClassIcon::ALL`] ends with it.
    Generic,
}

impl ClassIcon {
    /// Every class icon, in declaration order (tests paint each one and check the order).
    pub const ALL: [ClassIcon; 39] = [
        ClassIcon::Workspace,
        ClassIcon::Folder,
        ClassIcon::Model,
        ClassIcon::Block,
        ClassIcon::Ball,
        ClassIcon::Cylinder,
        ClassIcon::Wedge,
        ClassIcon::Spawn,
        ClassIcon::Checkpoint,
        ClassIcon::Script,
        ClassIcon::LocalScript,
        ClassIcon::ModuleScript,
        ClassIcon::RemoteEvent,
        ClassIcon::UnreliableRemoteEvent,
        ClassIcon::RemoteFunction,
        ClassIcon::Humanoid,
        ClassIcon::Player,
        ClassIcon::Players,
        ClassIcon::Gui,
        ClassIcon::Frame,
        ClassIcon::TextLabel,
        ClassIcon::Button,
        ClassIcon::ImageButton,
        ClassIcon::Lighting,
        ClassIcon::Sky,
        ClassIcon::Camera,
        ClassIcon::Atmosphere,
        ClassIcon::Clouds,
        ClassIcon::Bloom,
        ClassIcon::ColorCorrection,
        ClassIcon::ColorGrading,
        ClassIcon::Storage,
        ClassIcon::Server,
        ClassIcon::ScriptFolder,
        ClassIcon::RunService,
        ClassIcon::InputContext,
        ClassIcon::InputAction,
        ClassIcon::InputBinding,
        ClassIcon::Generic,
    ];
}

/// The icon of `class`: by name, then by its nearest known superclass, else
/// [`ClassIcon::Generic`]. `shape` is a part's `Shape` property ("Block", "Ball", "Cylinder",
/// "Wedge"); it only changes the icon of plain parts.
pub fn class_icon(class: &str, shape: Option<&str>) -> ClassIcon {
    use ClassIcon as C;
    let by_name = match class {
        "Workspace" => Some(C::Workspace),
        "Folder" => Some(C::Folder),
        "Model" => Some(C::Model),
        "SpawnLocation" => Some(C::Spawn),
        "Checkpoint" => Some(C::Checkpoint),
        "Script" => Some(C::Script),
        "LocalScript" => Some(C::LocalScript),
        "ModuleScript" => Some(C::ModuleScript),
        "RemoteEvent" => Some(C::RemoteEvent),
        "UnreliableRemoteEvent" => Some(C::UnreliableRemoteEvent),
        "RemoteFunction" => Some(C::RemoteFunction),
        "Humanoid" => Some(C::Humanoid),
        "Player" | "StarterPlayer" => Some(C::Player),
        "Players" => Some(C::Players),
        "ScreenGui" | "StarterGui" | "PlayerGui" => Some(C::Gui),
        "Frame" => Some(C::Frame),
        "TextLabel" => Some(C::TextLabel),
        "ImageButton" => Some(C::ImageButton),
        "Lighting" => Some(C::Lighting),
        "Sky" => Some(C::Sky),
        "Camera" => Some(C::Camera),
        "Atmosphere" => Some(C::Atmosphere),
        "Clouds" => Some(C::Clouds),
        "BloomEffect" => Some(C::Bloom),
        "ColorCorrectionEffect" => Some(C::ColorCorrection),
        "ColorGradingEffect" => Some(C::ColorGrading),
        "ReplicatedStorage" | "ServerStorage" => Some(C::Storage),
        "ServerScriptService" => Some(C::Server),
        "StarterPlayerScripts" | "PlayerScripts" => Some(C::ScriptFolder),
        "RunService" => Some(C::RunService),
        "InputContext" => Some(C::InputContext),
        "InputAction" => Some(C::InputAction),
        "InputBinding" => Some(C::InputBinding),
        // Abstract classes have no instances to show; these keep every class drawable.
        "PVInstance" => Some(C::Model),
        "GuiBase2d" => Some(C::Gui),
        "GuiObject" => Some(C::Frame),
        "LuaSourceContainer" | "BaseScript" => Some(C::Script),
        "BaseRemoteEvent" => Some(C::RemoteEvent),
        "ServiceProvider" | "DataModel" => Some(C::Workspace),
        _ => None,
    };
    let icon = by_name.unwrap_or_else(|| {
        // Most specific superclass first.
        [
            ("GuiButton", C::Button),
            ("Frame", C::Frame),
            ("Script", C::Script),
            ("BasePart", C::Block),
        ]
        .into_iter()
        .find(|(base, _)| is_subclass(class, base))
        .map_or(C::Generic, |(_, icon)| icon)
    });
    match (icon, shape) {
        (C::Block, Some("Ball")) => C::Ball,
        (C::Block, Some("Cylinder")) => C::Cylinder,
        (C::Block, Some("Wedge")) => C::Wedge,
        _ => icon,
    }
}

/// Paints the Explorer icon of `class` centred in `rect` (parts as blocks; see [`class_icon`]
/// for their other shapes).
pub fn paint_class(painter: &egui::Painter, rect: Rect, class: &str, c: Color32) {
    paint_class_icon(painter, rect, class_icon(class, None), c);
}

/// A page outline with its bottom right corner left open for a badge, and two lines of text.
fn page_with_badge(painter: &egui::Painter, o: Pos2, s: Stroke, fine: Stroke) {
    poly(
        painter,
        o,
        &[
            (5.0, 0.5),
            (5.0, -6.0),
            (-5.0, -6.0),
            (-5.0, 6.0),
            (0.5, 6.0),
        ],
        false,
        s,
    );
    line(painter, o, (-2.5, -3.0), (2.5, -3.0), fine);
    line(painter, o, (-2.5, -0.3), (2.5, -0.3), fine);
    line(painter, o, (-2.5, 2.4), (-0.5, 2.4), fine);
}

/// A bust: a head of radius `head` above shoulders of radius `shoulders`, `gap` apart.
fn bust(painter: &egui::Painter, at: Pos2, head: f32, shoulders: f32, gap: f32, s: Stroke) {
    let head_y = -3.0 - (head - 2.3);
    painter.circle_stroke(at + vec2(0.0, head_y), head, s);
    let top = head_y + head + gap;
    painter.add(Shape::line(
        arc(at + vec2(0.0, top + shoulders), shoulders, PI, TAU, 10),
        s,
    ));
}

/// Paints a class icon centred in `rect`.
pub fn paint_class_icon(painter: &egui::Painter, rect: Rect, icon: ClassIcon, c: Color32) {
    let o = rect.center();
    let s = Stroke::new(1.3, c);
    let fine = Stroke::new(1.0, c);
    let sq = |x: f32, y: f32, size: f32| Rect::from_center_size(o + vec2(x, y), Vec2::splat(size));
    match icon {
        ClassIcon::Workspace => paint(painter, rect, Icon::Space, c),
        ClassIcon::Folder => {
            let body = Rect::from_center_size(o + vec2(0.0, 1.0), vec2(12.0, 8.5));
            outline(painter, body, 1.0, s);
            painter.line_segment(
                [
                    body.left_top() + vec2(0.5, -2.0),
                    body.left_top() + vec2(5.0, -2.0),
                ],
                s,
            );
        }
        ClassIcon::ScriptFolder => {
            paint_class_icon(painter, rect, ClassIcon::Folder, c);
            line(painter, o, (-3.0, 0.5), (3.0, 0.5), fine);
            line(painter, o, (-3.0, 3.0), (1.0, 3.0), fine);
        }
        ClassIcon::Model => {
            corners(painter, o, 11.0, 4.0, s);
            painter.rect_filled(sq(0.0, 0.0, 4.0), 0.5, c);
        }
        ClassIcon::Block => {
            // A cube from above: the top face filled, the two sides outlined.
            let (w, h) = (5.8, 3.3);
            filled(painter, o, &[(0.0, -6.6), (w, -h), (0.0, 0.0), (-w, -h)], c);
            poly(
                painter,
                o,
                &[(-w, -h), (-w, h), (0.0, 6.6), (w, h), (w, -h)],
                false,
                s,
            );
            line(painter, o, (0.0, 0.0), (0.0, 6.6), s);
        }
        ClassIcon::Ball => {
            painter.circle_stroke(o, 6.2, s);
            painter.add(Shape::line(
                ellipse_arc(o, vec2(6.2, 2.4), 0.0, PI, 12),
                fine,
            ));
        }
        ClassIcon::Cylinder => {
            let (rx, ry, h) = (5.5, 2.0, 4.5);
            painter.add(Shape::closed_line(
                ellipse_arc(o + vec2(0.0, -h), vec2(rx, ry), 0.0, TAU, 20),
                s,
            ));
            line(painter, o, (-rx, -h), (-rx, h), s);
            line(painter, o, (rx, -h), (rx, h), s);
            painter.add(Shape::line(
                ellipse_arc(o + vec2(0.0, h), vec2(rx, ry), 0.0, PI, 12),
                s,
            ));
        }
        ClassIcon::Wedge => {
            filled(painter, o, &[(6.0, -6.0), (6.0, 5.5), (-6.5, 5.5)], c);
        }
        ClassIcon::Spawn => {
            painter.rect_filled(
                Rect::from_min_max(o + vec2(-6.5, 3.8), o + vec2(6.5, 6.8)),
                1.0,
                c,
            );
            line(painter, o, (0.0, 1.8), (0.0, -2.5), s);
            head_sized(painter, o + vec2(0.0, -6.8), vec2(0.0, -1.0), 4.5, 3.2, c);
        }
        ClassIcon::Checkpoint => {
            line(painter, o, (-4.5, -7.0), (-4.5, 7.0), s);
            filled(painter, o, &[(-4.5, -7.0), (6.0, -4.2), (-4.5, -1.4)], c);
        }
        ClassIcon::Script => {
            let page = Rect::from_center_size(o, vec2(10.0, 12.0));
            outline(painter, page, 1.0, s);
            for i in 0..3 {
                let y = page.top() + 3.5 + i as f32 * 2.7;
                painter.line_segment(
                    [pos2(page.left() + 2.5, y), pos2(page.right() - 2.5, y)],
                    fine,
                );
            }
        }
        ClassIcon::LocalScript => {
            page_with_badge(painter, o, s, fine);
            painter.circle_filled(o + vec2(4.3, 4.3), 2.6, c);
        }
        ClassIcon::ModuleScript => {
            page_with_badge(painter, o, s, fine);
            filled(
                painter,
                o,
                &[(4.3, 0.9), (7.4, 4.3), (4.3, 7.4), (1.2, 4.3)],
                c,
            );
        }
        ClassIcon::RemoteEvent => {
            line(painter, o, (-6.0, -2.0), (5.0, -2.0), s);
            head(painter, o + vec2(6.0, -2.0), vec2(1.0, 0.0), c);
            line(painter, o, (6.0, 3.0), (-5.0, 3.0), s);
            head(painter, o + vec2(-6.0, 3.0), vec2(-1.0, 0.0), c);
        }
        ClassIcon::UnreliableRemoteEvent => {
            dashed(painter, &[o + vec2(-6.5, -2.0), o + vec2(3.0, -2.0)], c);
            head(painter, o + vec2(6.0, -2.0), vec2(1.0, 0.0), c);
            dashed(painter, &[o + vec2(6.5, 3.0), o + vec2(-3.0, 3.0)], c);
            head(painter, o + vec2(-6.0, 3.0), vec2(-1.0, 0.0), c);
        }
        ClassIcon::RemoteFunction => {
            let mut pts = vec![o + vec2(-6.5, -3.5)];
            pts.extend(arc(o + vec2(2.5, 0.0), 3.5, -PI / 2.0, PI / 2.0, 10));
            pts.push(o + vec2(-3.5, 3.5));
            painter.add(Shape::line(pts, s));
            head_sized(painter, o + vec2(-7.0, 3.5), vec2(-1.0, 0.0), 4.0, 3.0, c);
        }
        ClassIcon::Humanoid => {
            painter.circle_stroke(o + vec2(0.0, -5.0), 2.0, s);
            line(painter, o, (0.0, -2.8), (0.0, 2.5), s);
            poly(
                painter,
                o,
                &[(-4.5, 0.5), (0.0, -1.5), (4.5, 0.5)],
                false,
                s,
            );
            poly(painter, o, &[(-3.5, 7.0), (0.0, 2.5), (3.5, 7.0)], false, s);
        }
        ClassIcon::Player => bust(painter, o, 2.7, 5.0, 2.2, s),
        ClassIcon::Players => {
            for dx in [-3.5, 3.5] {
                bust(painter, o + vec2(dx, 0.0), 2.3, 3.8, 2.4, s);
            }
        }
        ClassIcon::Gui => {
            let win = sq(0.0, 0.0, 14.0).shrink2(vec2(0.0, 1.5));
            outline(painter, win, 1.5, s);
            line(painter, o, (-7.0, -2.0), (7.0, -2.0), s);
            painter.circle_filled(o + vec2(-4.5, -3.8), 0.8, c);
        }
        ClassIcon::Frame => {
            outline(painter, Rect::from_center_size(o, vec2(13.0, 9.0)), 1.5, s);
        }
        ClassIcon::TextLabel => {
            outline(painter, Rect::from_center_size(o, vec2(13.0, 9.0)), 1.5, s);
            line(painter, o, (-3.5, -1.2), (3.5, -1.2), fine);
            line(painter, o, (-3.5, 1.5), (1.0, 1.5), fine);
        }
        ClassIcon::Button => {
            outline(painter, Rect::from_center_size(o, vec2(14.0, 8.0)), 4.0, s);
            line(painter, o, (-3.0, 0.0), (3.0, 0.0), s);
        }
        ClassIcon::ImageButton => {
            outline(painter, Rect::from_center_size(o, vec2(13.0, 11.0)), 2.0, s);
            poly(
                painter,
                o,
                &[(-5.0, 4.0), (-1.5, 0.0), (1.0, 2.5), (2.5, 1.0), (5.0, 4.0)],
                false,
                fine,
            );
            painter.circle_filled(o + vec2(2.8, -2.0), 1.2, c);
        }
        ClassIcon::Lighting => {
            painter.circle_stroke(o + vec2(0.0, -2.0), 4.5, s);
            line(painter, o, (-2.0, 2.0), (-2.0, 4.2), s);
            line(painter, o, (2.0, 2.0), (2.0, 4.2), s);
            line(painter, o, (-2.5, 4.5), (2.5, 4.5), s);
            line(painter, o, (-1.5, 6.6), (1.5, 6.6), s);
        }
        ClassIcon::Sky => {
            line(painter, o, (-7.0, 4.0), (7.0, 4.0), s);
            painter.add(Shape::line(arc(o + vec2(0.0, 4.0), 4.0, PI, TAU, 12), s));
            // Three short rays above the rising sun.
            for a in [-PI / 2.0, -PI / 4.0, -3.0 * PI / 4.0] {
                let d = vec2(a.cos(), a.sin());
                let base = o + vec2(0.0, 4.0);
                painter.line_segment([base + d * 5.6, base + d * 7.4], fine);
            }
            line(painter, o, (-5.0, 6.8), (5.0, 6.8), fine);
        }
        ClassIcon::Camera => {
            outline(
                painter,
                Rect::from_center_size(o + vec2(-1.5, 0.0), vec2(10.0, 8.0)),
                1.5,
                s,
            );
            poly(
                painter,
                o,
                &[(3.5, -1.5), (7.0, -3.5), (7.0, 3.5), (3.5, 1.5)],
                true,
                s,
            );
        }
        ClassIcon::Atmosphere => {
            for base in [-4.5f32, 0.0, 4.5] {
                let pts: Vec<Pos2> = (0..=12)
                    .map(|i| {
                        let x = -6.5 + i as f32 * 13.0 / 12.0;
                        o + vec2(x, base + 0.9 * (x * 0.95).sin())
                    })
                    .collect();
                painter.add(Shape::line(pts, s));
            }
        }
        ClassIcon::Clouds => {
            painter.circle_filled(o + vec2(-3.0, 1.0), 3.0, c);
            painter.circle_filled(o + vec2(1.6, -1.2), 3.8, c);
            painter.circle_filled(o + vec2(-5.4, 3.0), 1.8, c);
            painter.circle_filled(o + vec2(5.2, 2.0), 2.4, c);
            painter.rect_filled(
                Rect::from_min_max(o + vec2(-5.8, 2.0), o + vec2(6.0, 4.8)),
                1.5,
                c,
            );
        }
        ClassIcon::Bloom => {
            painter.circle_filled(o, 2.6, c);
            painter.circle_stroke(o, 5.8, fine);
        }
        ClassIcon::ColorCorrection => {
            painter.circle_stroke(o, 6.0, s);
            let mut half = arc(o, 6.0, -PI / 2.0, PI / 2.0, 12);
            half.push(o + vec2(0.0, -6.0));
            painter.add(Shape::convex_polygon(half, c, Stroke::NONE));
        }
        ClassIcon::ColorGrading => {
            for (y, knob) in [(-4.5f32, -2.5f32), (0.0, 3.0), (4.5, -0.5)] {
                line(painter, o, (-6.5, y), (6.5, y), fine);
                painter.circle_filled(o + vec2(knob, y), 1.9, c);
            }
        }
        ClassIcon::Storage => {
            outline(
                painter,
                Rect::from_min_max(o + vec2(-7.0, -6.0), o + vec2(7.0, -2.5)),
                0.5,
                s,
            );
            poly(
                painter,
                o,
                &[(-6.0, -2.5), (-6.0, 6.0), (6.0, 6.0), (6.0, -2.5)],
                false,
                s,
            );
            line(painter, o, (-2.0, 0.8), (2.0, 0.8), s);
        }
        ClassIcon::Server => {
            for y in [-3.4f32, 3.4] {
                outline(
                    painter,
                    Rect::from_center_size(o + vec2(0.0, y), vec2(13.0, 5.6)),
                    1.0,
                    s,
                );
                painter.circle_filled(o + vec2(3.8, y), 0.9, c);
                line(painter, o, (-4.0, y), (-1.0, y), fine);
            }
        }
        ClassIcon::RunService => {
            painter.circle_stroke(o + vec2(0.0, 1.0), 5.5, s);
            line(painter, o, (0.0, 1.0), (0.0, -2.2), s);
            line(painter, o, (0.0, 1.0), (2.4, 2.4), s);
            line(painter, o, (-1.8, -6.8), (1.8, -6.8), s);
            line(painter, o, (0.0, -6.8), (0.0, -4.5), s);
        }
        ClassIcon::InputContext => {
            outline(painter, Rect::from_center_size(o, vec2(14.5, 9.0)), 3.5, s);
            line(painter, o, (-5.4, 0.0), (-1.8, 0.0), s);
            line(painter, o, (-3.6, -1.8), (-3.6, 1.8), s);
            painter.circle_filled(o + vec2(2.6, 1.0), 1.0, c);
            painter.circle_filled(o + vec2(4.6, -1.0), 1.0, c);
        }
        ClassIcon::InputAction => {
            // A bolt: something fires.
            poly(
                painter,
                o,
                &[
                    (2.0, -7.0),
                    (-4.0, 1.0),
                    (-0.2, 1.0),
                    (-2.0, 7.0),
                    (4.0, -1.0),
                    (0.2, -1.0),
                ],
                true,
                s,
            );
        }
        ClassIcon::InputBinding => {
            outline(painter, sq(0.0, 0.0, 13.0), 2.5, s);
            outline(painter, sq(0.0, -1.2, 7.0), 1.0, fine);
        }
        ClassIcon::Generic => {
            outline(painter, sq(0.0, 0.0, 10.0), 3.0, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_core::classes::registry_classes;

    /// Paints with `f` into a fresh context and returns the bounds of what was drawn.
    fn painted_bounds(f: impl Fn(&egui::Painter, Rect)) -> Rect {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(20.0, 20.0), Vec2::splat(16.0));
        let mut out = ctx.run_pass(Default::default(), |ctx| {
            f(&ctx.layer_painter(egui::LayerId::background()), rect);
        });
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty(), "nothing painted");
        out.shapes
            .iter()
            .map(|s| s.shape.visual_bounding_rect())
            .fold(Rect::NOTHING, |a, b| a.union(b))
    }

    #[test]
    fn every_icon_paints_inside_16px() {
        for (n, icon) in Icon::ALL.iter().enumerate() {
            assert_eq!(*icon as usize, n, "Icon::ALL is out of order at {icon:?}");
        }
        assert_eq!(Icon::More as usize + 1, Icon::ALL.len());
        for (n, icon) in ClassIcon::ALL.iter().enumerate() {
            assert_eq!(
                *icon as usize, n,
                "ClassIcon::ALL is out of order at {icon:?}"
            );
        }
        assert_eq!(ClassIcon::Generic as usize + 1, ClassIcon::ALL.len());
        let slack = Rect::from_min_size(pos2(20.0, 20.0), Vec2::splat(16.0)).expand(1.0);
        for icon in Icon::ALL {
            let b = painted_bounds(|p, r| paint(p, r, icon, Color32::WHITE));
            assert!(slack.contains_rect(b), "{icon:?} spills out: {b:?}");
        }
        for icon in ClassIcon::ALL {
            let b = painted_bounds(|p, r| paint_class_icon(p, r, icon, Color32::WHITE));
            assert!(slack.contains_rect(b), "{icon:?} spills out: {b:?}");
        }
    }

    #[test]
    fn every_registry_class_has_its_own_icon() {
        for def in registry_classes() {
            let icon = class_icon(def.name, None);
            if def.name == "Instance" {
                assert_eq!(icon, ClassIcon::Generic);
            } else {
                assert_ne!(icon, ClassIcon::Generic, "{} has no icon", def.name);
            }
        }
        // The issue's list (#476): each one has an icon of its own.
        let named = [
            ("Part", ClassIcon::Block),
            ("Model", ClassIcon::Model),
            ("Folder", ClassIcon::Folder),
            ("SpawnLocation", ClassIcon::Spawn),
            ("Checkpoint", ClassIcon::Checkpoint),
            ("Script", ClassIcon::Script),
            ("LocalScript", ClassIcon::LocalScript),
            ("ModuleScript", ClassIcon::ModuleScript),
            ("RemoteEvent", ClassIcon::RemoteEvent),
            ("RemoteFunction", ClassIcon::RemoteFunction),
            ("Humanoid", ClassIcon::Humanoid),
            ("Player", ClassIcon::Player),
            ("Players", ClassIcon::Players),
            ("ScreenGui", ClassIcon::Gui),
            ("Frame", ClassIcon::Frame),
            ("TextLabel", ClassIcon::TextLabel),
            ("TextButton", ClassIcon::Button),
            ("ImageButton", ClassIcon::ImageButton),
            ("Lighting", ClassIcon::Lighting),
            ("BloomEffect", ClassIcon::Bloom),
            ("ServerScriptService", ClassIcon::Server),
        ];
        for (class, icon) in named {
            assert_eq!(class_icon(class, None), icon, "{class}");
        }
    }

    #[test]
    fn parts_follow_shape_and_superclass() {
        assert_eq!(class_icon("Part", Some("Ball")), ClassIcon::Ball);
        assert_eq!(class_icon("Part", Some("Cylinder")), ClassIcon::Cylinder);
        assert_eq!(class_icon("Part", Some("Wedge")), ClassIcon::Wedge);
        assert_eq!(class_icon("Part", Some("Block")), ClassIcon::Block);
        assert_eq!(class_icon("BasePart", None), ClassIcon::Block);
        // A spawn stays a spawn whatever its shape.
        assert_eq!(class_icon("SpawnLocation", Some("Ball")), ClassIcon::Spawn);
        // By superclass: GuiButton is a Frame, but buttons come first.
        assert_eq!(class_icon("GuiButton", None), ClassIcon::Button);
        assert_eq!(class_icon("NotAClass", None), ClassIcon::Generic);
    }
}
