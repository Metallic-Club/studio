//! Selection modes and Select similar (spec 10.1, issue #220).
//!
//! The viewport projects each candidate part to the screen and passes [`ScreenItem`]s; these
//! functions decide membership and how a gesture combines with the current selection. The
//! selection is an ordered list of ids whose last entry is the active item (spec 2.3).
//!
//! - **Box**: parts whose screen rectangle lies fully inside the box; with Alt, parts that
//!   touch it too ([`box_hits`]).
//! - **Lasso**: parts whose screen-space centre is inside the polygon ([`lasso_hits`]).
//! - **Paint**: parts the 24 px brush circle passes over ([`paint_hits`]).
//! - Shift adds, Ctrl removes, otherwise the gesture replaces ([`SelectOp`], [`combine`]).
//! - **Select similar** (Shift+G): [`select_similar`] with a [`SimilarKey`].

use std::collections::HashSet;
use std::hash::Hash;

use glam::{DVec2, DVec3};

/// Diameter of the paint-select circle in screen pixels.
pub const PAINT_BRUSH_DIAMETER_PX: f64 = 24.0;

/// Mode of the Select tool. Pressing Q while the tool is active cycles Box, Lasso, Paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectMode {
    /// Rectangle drag.
    #[default]
    Box,
    /// Freehand polygon.
    Lasso,
    /// Brush circle.
    Paint,
}

impl SelectMode {
    /// The next mode in the Q cycle.
    pub fn next(self) -> Self {
        match self {
            SelectMode::Box => SelectMode::Lasso,
            SelectMode::Lasso => SelectMode::Paint,
            SelectMode::Paint => SelectMode::Box,
        }
    }

    /// Name shown in the viewport header.
    pub fn label(self) -> &'static str {
        match self {
            SelectMode::Box => "Box",
            SelectMode::Lasso => "Lasso",
            SelectMode::Paint => "Paint",
        }
    }
}

/// How a selection gesture combines with the current selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectOp {
    /// The hits become the selection.
    #[default]
    Replace,
    /// Shift: add the hits.
    Add,
    /// Ctrl: remove the hits.
    Subtract,
}

impl SelectOp {
    /// The op for the modifiers held when the gesture ends. Ctrl wins over Shift.
    pub fn from_modifiers(shift: bool, ctrl: bool) -> Self {
        if ctrl {
            SelectOp::Subtract
        } else if shift {
            SelectOp::Add
        } else {
            SelectOp::Replace
        }
    }
}

/// An axis-aligned rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// Top-left corner (smallest x and y).
    pub min: DVec2,
    /// Bottom-right corner.
    pub max: DVec2,
}

impl ScreenRect {
    /// The rectangle spanned by two corners in any order (a drag's start and end).
    pub fn from_corners(a: DVec2, b: DVec2) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// Whether `other` lies fully inside this rectangle.
    pub fn contains_rect(&self, other: &ScreenRect) -> bool {
        other.min.cmpge(self.min).all() && other.max.cmple(self.max).all()
    }

    /// Whether the rectangles overlap or touch.
    pub fn intersects(&self, other: &ScreenRect) -> bool {
        other.min.cmple(self.max).all() && other.max.cmpge(self.min).all()
    }

    /// Whether a point is inside or on the edge.
    pub fn contains_point(&self, p: DVec2) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// Distance from a point to the rectangle (0 inside).
    pub fn distance_to_point(&self, p: DVec2) -> f64 {
        let d = (self.min - p).max(p - self.max).max(DVec2::ZERO);
        d.length()
    }
}

/// A candidate part as the viewport sees it this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenItem<I> {
    /// The instance.
    pub id: I,
    /// Screen rectangle of its projected bounding box.
    pub rect: ScreenRect,
    /// Screen position of its projected centre.
    pub centre: DVec2,
    /// `Locked` parts cannot be picked in the viewport (Explorer still selects them).
    pub locked: bool,
}

/// Parts hit by a box. Fully inside, or touching when `include_touching` (Alt) is held.
pub fn box_hits<I: Copy>(
    items: &[ScreenItem<I>],
    area: ScreenRect,
    include_touching: bool,
) -> Vec<I> {
    items
        .iter()
        .filter(|it| !it.locked)
        .filter(|it| {
            if include_touching {
                area.intersects(&it.rect)
            } else {
                area.contains_rect(&it.rect)
            }
        })
        .map(|it| it.id)
        .collect()
}

/// Whether `p` is inside a polygon (even-odd rule). Fewer than three points contain nothing.
pub fn point_in_polygon(p: DVec2, polygon: &[DVec2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Parts whose screen-space centre lies inside the lasso polygon.
pub fn lasso_hits<I: Copy>(items: &[ScreenItem<I>], polygon: &[DVec2]) -> Vec<I> {
    // Bounding rectangle first: most parts fail it cheaply.
    let Some(first) = polygon.first() else {
        return Vec::new();
    };
    let bounds = polygon
        .iter()
        .fold(ScreenRect::from_corners(*first, *first), |r, p| {
            ScreenRect::from_corners(r.min.min(*p), r.max.max(*p))
        });
    items
        .iter()
        .filter(|it| !it.locked && bounds.contains_point(it.centre))
        .filter(|it| point_in_polygon(it.centre, polygon))
        .map(|it| it.id)
        .collect()
}

/// Shortest distance between a segment and a rectangle (0 when they meet).
fn segment_rect_distance(a: DVec2, b: DVec2, r: &ScreenRect) -> f64 {
    if r.contains_point(a) || r.contains_point(b) {
        return 0.0;
    }
    let corners = [
        r.min,
        DVec2::new(r.max.x, r.min.y),
        r.max,
        DVec2::new(r.min.x, r.max.y),
    ];
    let mut best = r.distance_to_point(a).min(r.distance_to_point(b));
    for k in 0..4 {
        let (c, d) = (corners[k], corners[(k + 1) % 4]);
        if segments_cross(a, b, c, d) {
            return 0.0;
        }
        best = best.min(point_segment_distance(c, a, b));
    }
    best
}

fn point_segment_distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    let t = if len_sq == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    };
    p.distance(a + ab * t)
}

fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let o = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (o(c, d, a), o(c, d, b));
    let (d3, d4) = (o(a, b, c), o(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// Parts the paint circle passed over. `path` is the pointer trail in screen pixels; the
/// circle has [`PAINT_BRUSH_DIAMETER_PX`]. A part is hit when its screen rectangle comes
/// within the circle's radius of the trail.
pub fn paint_hits<I: Copy>(items: &[ScreenItem<I>], path: &[DVec2]) -> Vec<I> {
    let radius = PAINT_BRUSH_DIAMETER_PX * 0.5;
    items
        .iter()
        .filter(|it| !it.locked)
        .filter(|it| match path {
            [] => false,
            [p] => it.rect.distance_to_point(*p) <= radius,
            _ => path
                .windows(2)
                .any(|w| segment_rect_distance(w[0], w[1], &it.rect) <= radius),
        })
        .map(|it| it.id)
        .collect()
}

/// Combines a gesture's hits with the current selection. Order is kept; with Replace and Add
/// the last hit becomes the active item (the list's last entry).
pub fn combine<I: Copy + Eq + Hash>(current: &[I], hits: &[I], op: SelectOp) -> Vec<I> {
    match op {
        SelectOp::Replace => dedup(hits),
        SelectOp::Add => {
            let hit_set: HashSet<I> = hits.iter().copied().collect();
            let mut out: Vec<I> = current
                .iter()
                .copied()
                .filter(|id| !hit_set.contains(id))
                .collect();
            out.extend(dedup(hits));
            out
        }
        SelectOp::Subtract => {
            let hit_set: HashSet<I> = hits.iter().copied().collect();
            current
                .iter()
                .copied()
                .filter(|id| !hit_set.contains(id))
                .collect()
        }
    }
}

fn dedup<I: Copy + Eq + Hash>(ids: &[I]) -> Vec<I> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// Every id in `all` that is not selected ("Invert selection").
pub fn invert<I: Copy + Eq + Hash>(all: &[I], current: &[I]) -> Vec<I> {
    let selected: HashSet<I> = current.iter().copied().collect();
    all.iter()
        .copied()
        .filter(|id| !selected.contains(id))
        .collect()
}

/// What Select similar compares (Shift+G, then the key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimilarKey {
    /// Same colour (C): equal after rounding each channel to 0..=255.
    Color,
    /// Same material (M).
    Material,
    /// Same per-part style, `LookOverride` (L).
    Style,
    /// Same size (S): every axis equal within [`SIZE_TOLERANCE`].
    Size,
    /// Same class (K).
    Class,
    /// Same name (N).
    Name,
    /// Same set of tags (T).
    Tags,
    /// Same parent (P).
    Parent,
}

/// Size difference treated as equal, in studs.
pub const SIZE_TOLERANCE: f64 = 1e-4;

impl SimilarKey {
    /// All keys in menu order.
    pub const ALL: [SimilarKey; 8] = [
        SimilarKey::Color,
        SimilarKey::Material,
        SimilarKey::Style,
        SimilarKey::Size,
        SimilarKey::Class,
        SimilarKey::Name,
        SimilarKey::Tags,
        SimilarKey::Parent,
    ];

    /// The menu accelerator key.
    pub fn key(self) -> char {
        match self {
            SimilarKey::Color => 'C',
            SimilarKey::Material => 'M',
            SimilarKey::Style => 'L',
            SimilarKey::Size => 'S',
            SimilarKey::Class => 'K',
            SimilarKey::Name => 'N',
            SimilarKey::Tags => 'T',
            SimilarKey::Parent => 'P',
        }
    }

    /// The key for an accelerator letter (either case).
    pub fn from_key(c: char) -> Option<Self> {
        let c = c.to_ascii_uppercase();
        Self::ALL.into_iter().find(|k| k.key() == c)
    }

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            SimilarKey::Color => "Same Color",
            SimilarKey::Material => "Same Material",
            SimilarKey::Style => "Same Style",
            SimilarKey::Size => "Same Size",
            SimilarKey::Class => "Same Class",
            SimilarKey::Name => "Same Name",
            SimilarKey::Tags => "Same Tags",
            SimilarKey::Parent => "Same Parent",
        }
    }
}

/// What Select similar knows about one instance (a snapshot the caller reads from the world).
#[derive(Debug, Clone, PartialEq)]
pub struct PartTraits<I> {
    /// The instance.
    pub id: I,
    /// Class name.
    pub class: String,
    /// `Name`.
    pub name: String,
    /// `Color`, channels 0 to 255.
    pub color: [f64; 3],
    /// `Material`.
    pub material: String,
    /// `LookOverride`; `None` means inherit.
    pub style: Option<String>,
    /// `Size` in studs.
    pub size: DVec3,
    /// Tags.
    pub tags: Vec<String>,
    /// Parent instance.
    pub parent: Option<I>,
}

/// Hashable value of one trait, so matching is one hash lookup per candidate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TraitValue<I> {
    Text(String),
    Style(Option<String>),
    Ints([i64; 3]),
    Tags(Vec<String>),
    Parent(Option<I>),
}

fn trait_value<I: Copy>(t: &PartTraits<I>, key: SimilarKey) -> TraitValue<I> {
    match key {
        SimilarKey::Color => TraitValue::Ints(t.color.map(|c| c.round().clamp(0.0, 255.0) as i64)),
        SimilarKey::Material => TraitValue::Text(t.material.clone()),
        SimilarKey::Style => TraitValue::Style(t.style.clone()),
        SimilarKey::Size => {
            let q = |v: f64| (v / SIZE_TOLERANCE).round() as i64;
            TraitValue::Ints([q(t.size.x), q(t.size.y), q(t.size.z)])
        }
        SimilarKey::Class => TraitValue::Text(t.class.clone()),
        SimilarKey::Name => TraitValue::Text(t.name.clone()),
        SimilarKey::Tags => {
            let mut tags = t.tags.clone();
            tags.sort();
            tags.dedup();
            TraitValue::Tags(tags)
        }
        SimilarKey::Parent => TraitValue::Parent(t.parent),
    }
}

/// Select similar: every part in `parts` that matches any selected part on `key`, in `parts`
/// order (the selected parts themselves included). `within` limits the result to the ids
/// inside the current box when one is drawn; `None` searches the whole place.
pub fn select_similar<I: Copy + Eq + Hash>(
    parts: &[PartTraits<I>],
    selected: &[I],
    key: SimilarKey,
    within: Option<&[I]>,
) -> Vec<I> {
    let selected: HashSet<I> = selected.iter().copied().collect();
    let wanted: HashSet<TraitValue<I>> = parts
        .iter()
        .filter(|p| selected.contains(&p.id))
        .map(|p| trait_value(p, key))
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let scope: Option<HashSet<I>> = within.map(|ids| ids.iter().copied().collect());
    parts
        .iter()
        .filter(|p| scope.as_ref().is_none_or(|s| s.contains(&p.id)))
        .filter(|p| wanted.contains(&trait_value(p, key)))
        .map(|p| p.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> ScreenRect {
        ScreenRect::from_corners(DVec2::new(x0, y0), DVec2::new(x1, y1))
    }

    fn item(id: u32, r: ScreenRect) -> ScreenItem<u32> {
        ScreenItem {
            id,
            rect: r,
            centre: (r.min + r.max) * 0.5,
            locked: false,
        }
    }

    #[test]
    fn mode_cycles_box_lasso_paint() {
        let m = SelectMode::default();
        assert_eq!(m, SelectMode::Box);
        assert_eq!(m.next(), SelectMode::Lasso);
        assert_eq!(m.next().next(), SelectMode::Paint);
        assert_eq!(m.next().next().next(), SelectMode::Box);
        assert_eq!(SelectMode::Lasso.label(), "Lasso");
    }

    #[test]
    fn modifiers_pick_the_op() {
        assert_eq!(SelectOp::from_modifiers(false, false), SelectOp::Replace);
        assert_eq!(SelectOp::from_modifiers(true, false), SelectOp::Add);
        assert_eq!(SelectOp::from_modifiers(false, true), SelectOp::Subtract);
        assert_eq!(SelectOp::from_modifiers(true, true), SelectOp::Subtract);
    }

    #[test]
    fn box_inside_versus_touching() {
        let items = [
            item(1, rect(10.0, 10.0, 20.0, 20.0)),     // fully inside
            item(2, rect(90.0, 90.0, 120.0, 120.0)),   // straddles the edge
            item(3, rect(200.0, 200.0, 210.0, 210.0)), // outside
            ScreenItem {
                locked: true,
                ..item(4, rect(30.0, 30.0, 40.0, 40.0))
            },
        ];
        // Dragged from bottom-right to top-left: corner order does not matter.
        let area = ScreenRect::from_corners(DVec2::new(100.0, 100.0), DVec2::new(0.0, 0.0));
        assert_eq!(box_hits(&items, area, false), vec![1]);
        assert_eq!(box_hits(&items, area, true), vec![1, 2]);
    }

    /// Issue #220 "done when": lasso membership is decided by the screen-space centre.
    #[test]
    fn lasso_membership_by_centre() {
        // An L-shaped lasso: the notch at top right is outside.
        let lasso = [
            DVec2::new(0.0, 0.0),
            DVec2::new(50.0, 0.0),
            DVec2::new(50.0, 50.0),
            DVec2::new(100.0, 50.0),
            DVec2::new(100.0, 100.0),
            DVec2::new(0.0, 100.0),
        ];
        let items = [
            item(1, rect(10.0, 10.0, 20.0, 20.0)), // centre (15, 15): inside
            item(2, rect(70.0, 10.0, 80.0, 20.0)), // centre in the notch: outside
            item(3, rect(60.0, 60.0, 90.0, 90.0)), // centre (75, 75): inside
            // Rectangle sticks far outside, but the centre (45, 80) is inside.
            item(4, rect(-10.0, 60.0, 100.0, 100.0)),
            // Rectangle overlaps the lasso, but the centre (75, 25) is in the notch.
            item(5, rect(40.0, 0.0, 110.0, 50.0)),
            item(6, rect(300.0, 300.0, 310.0, 310.0)), // far away
        ];
        assert_eq!(lasso_hits(&items, &lasso), vec![1, 3, 4]);
        assert!(lasso_hits(&items, &lasso[..2]).is_empty());
        assert!(lasso_hits::<u32>(&items, &[]).is_empty());
    }

    #[test]
    fn point_in_polygon_handles_concave_and_triangles() {
        let tri = [DVec2::ZERO, DVec2::new(10.0, 0.0), DVec2::new(0.0, 10.0)];
        assert!(point_in_polygon(DVec2::new(2.0, 2.0), &tri));
        assert!(!point_in_polygon(DVec2::new(6.0, 6.0), &tri));
        assert!(!point_in_polygon(DVec2::new(-1.0, 1.0), &tri));
    }

    #[test]
    fn paint_select_hits_parts_near_the_trail() {
        let items = [
            item(1, rect(0.0, 20.0, 10.0, 30.0)), // 10 px under the trail: hit (radius 12)
            item(2, rect(50.0, 40.0, 60.0, 50.0)), // 30 px away: missed
            item(3, rect(95.0, -5.0, 105.0, 5.0)), // the trail crosses it: hit
            item(4, rect(200.0, 0.0, 210.0, 10.0)), // beyond the end, 100 px: missed
        ];
        let trail = [
            DVec2::new(0.0, 10.0),
            DVec2::new(100.0, 10.0),
            DVec2::new(100.0, -20.0),
        ];
        assert_eq!(paint_hits(&items, &trail), vec![1, 3]);
        // A click without movement is a single dab.
        assert_eq!(paint_hits(&items, &[DVec2::new(5.0, 15.0)]), vec![1]);
        assert!(paint_hits(&items, &[]).is_empty());
    }

    #[test]
    fn combine_replace_add_subtract() {
        let current = [1, 2, 3];
        assert_eq!(combine(&current, &[4, 2, 4], SelectOp::Replace), vec![4, 2]);
        // Add keeps order and makes the last hit active.
        assert_eq!(combine(&current, &[2, 5], SelectOp::Add), vec![1, 3, 2, 5]);
        assert_eq!(combine(&current, &[2, 9], SelectOp::Subtract), vec![1, 3]);
        assert_eq!(invert(&[1, 2, 3, 4, 5], &current), vec![4, 5]);
    }

    fn part(id: u32, f: impl FnOnce(&mut PartTraits<u32>)) -> PartTraits<u32> {
        let mut p = PartTraits {
            id,
            class: "Part".into(),
            name: "Part".into(),
            color: [163.0, 162.0, 165.0],
            material: "Plastic".into(),
            style: None,
            size: DVec3::new(4.0, 1.0, 2.0),
            tags: Vec::new(),
            parent: Some(100),
        };
        f(&mut p);
        p
    }

    /// One scene where each part differs from part 1 in exactly one trait, plus a twin.
    fn similar_scene() -> Vec<PartTraits<u32>> {
        vec![
            part(1, |p| p.tags = vec!["Wall".into(), "Red".into()]),
            part(2, |p| {
                p.color = [10.0, 20.0, 30.0];
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(3, |p| {
                p.material = "Brick".into();
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(4, |p| {
                p.style = Some("retro-32".into());
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(5, |p| {
                p.size = DVec3::new(4.0, 1.0, 2.5);
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(6, |p| {
                p.class = "SpawnLocation".into();
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(7, |p| {
                p.name = "Door".into();
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            part(8, |p| p.tags = vec!["Wall".into()]),
            part(9, |p| {
                p.parent = Some(200);
                p.tags = vec!["Wall".into(), "Red".into()];
            }),
            // Twin of 1 with tags in another order, a colour within rounding and a size within
            // tolerance.
            part(10, |p| {
                p.tags = vec!["Red".into(), "Wall".into(), "Red".into()];
                p.color = [163.2, 161.9, 165.4];
                p.size = DVec3::new(4.0 + 4e-5, 1.0, 2.0);
            }),
        ]
    }

    /// Issue #220 "done when": each Select similar key.
    #[test]
    fn each_similar_key() {
        let parts = similar_scene();
        let all: Vec<u32> = (1..=10).collect();
        let expect = |key: SimilarKey, differs: u32| {
            let got = select_similar(&parts, &[1], key, None);
            let want: Vec<u32> = all.iter().copied().filter(|id| *id != differs).collect();
            assert_eq!(got, want, "{}", key.label());
        };
        expect(SimilarKey::Color, 2);
        expect(SimilarKey::Material, 3);
        expect(SimilarKey::Style, 4);
        expect(SimilarKey::Size, 5);
        expect(SimilarKey::Class, 6);
        expect(SimilarKey::Name, 7);
        expect(SimilarKey::Tags, 8);
        expect(SimilarKey::Parent, 9);
    }

    #[test]
    fn similar_with_several_references_scope_and_empty() {
        let parts = similar_scene();
        // Materials Plastic (from 1) and Brick (from 3): everything.
        assert_eq!(
            select_similar(&parts, &[1, 3], SimilarKey::Material, None).len(),
            10
        );
        // Only inside the current box.
        assert_eq!(
            select_similar(&parts, &[1], SimilarKey::Name, Some(&[1, 2, 7, 8])),
            vec![1, 2, 8]
        );
        // No reference: nothing.
        assert!(select_similar(&parts, &[], SimilarKey::Class, None).is_empty());
        assert!(select_similar(&parts, &[999], SimilarKey::Class, None).is_empty());
    }

    #[test]
    fn similar_keys_map_to_menu_letters() {
        for k in SimilarKey::ALL {
            assert_eq!(SimilarKey::from_key(k.key()), Some(k));
            assert_eq!(SimilarKey::from_key(k.key().to_ascii_lowercase()), Some(k));
        }
        assert_eq!(SimilarKey::from_key('X'), None);
        let letters: HashSet<char> = SimilarKey::ALL.iter().map(|k| k.key()).collect();
        assert_eq!(letters.len(), 8);
    }
}
