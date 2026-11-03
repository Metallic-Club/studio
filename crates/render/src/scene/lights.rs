//! The scene's local lights: every `PointLight` and `SpotLight` in the world, kept current from the same change
//! consumer as the parts (see [`crate::lights`] for how they are drawn).
//!
//! A light is a dense record ([`SceneLight`]) with its world placement resolved: a light inside a part follows the
//! part (its `CFrame` is relative to the part's), anywhere else its `CFrame` is in world space, as `Light` says. The
//! part a light follows is its parent when the parent is drawn as a part (its class has a mesh component, which is
//! what `BasePart`s have); the scene never names a class. When such a part moves, the lights it holds are placed
//! again in the same sync: the part's index is stamped, and the lights whose part was stamped are re-read once.
//! Records are removed by swap, so nothing is ever scanned for a removal, and the renderer's per-light state
//! ([`LightRuntime`]) moves with its light.

use metallic_core::glam::Vec3;
use metallic_core::{CFrame, Color3, DataModel, DirtyBits, Id, PropId};

use super::components::{ClassTable, LightBinding};
use crate::lights::LightRuntime;

/// Which kind of local light an instance is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightKind {
    /// Light in every direction from a point (`PointLight`).
    Point,
    /// A cone of light along the light's forward direction (`SpotLight`).
    Spot,
}

impl LightKind {
    /// How many shadow-atlas tiles a shadowed light of this kind takes: a cube's six faces for a point light, one
    /// for a spot.
    pub const fn shadow_tiles(self) -> usize {
        match self {
            LightKind::Point => 6,
            LightKind::Spot => 1,
        }
    }
}

/// One local light as the renderer reads it: its `Light` properties and its place in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLight {
    /// The light instance.
    pub id: Id,
    /// Point or spot.
    pub kind: LightKind,
    /// Where it is, in world space (after the part it follows, if any).
    pub position: Vec3,
    /// Its forward direction (−Z of its world `CFrame`), unit length: where a spot light points.
    pub direction: Vec3,
    /// `Color`, sRGB-encoded as stored.
    pub color: Color3,
    /// `Brightness`.
    pub brightness: f32,
    /// `Color` decoded to linear light, times `Brightness`: what the light shines with (worked out when the light
    /// changes, not per frame).
    pub intensity: Vec3,
    /// `Range`, in metres: past it the light lights nothing.
    pub range: f32,
    /// `SpotLight.Angle`: the cone's full angle in degrees (0 for a point light).
    pub angle: f32,
    /// `Enabled`.
    pub enabled: bool,
    /// `Shadows`.
    pub shadows: bool,
    /// The part it follows, if any.
    pub host: Option<Id>,
}

impl SceneLight {
    /// Whether it lights anything: enabled, with some brightness, colour and range, and a finite place.
    pub fn shines(&self) -> bool {
        self.enabled
            && self.range > 0.0
            && self.range.is_finite()
            && self.intensity.max_element() > 0.0
            && self.intensity.is_finite()
            && self.position.is_finite()
            && self.direction.is_finite()
    }
}

/// Where a light reads its placement from: its own `CFrame` and, inside a part, the part's.
#[derive(Clone, Copy, Debug)]
struct Link {
    cframe: PropId,
    host_cframe: Option<PropId>,
}

/// An index of [`LightSet::index`] with no light.
const NONE: u32 = u32::MAX;

/// What a change did to a light, for [`super::SyncStats::lights`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightChange {
    None,
    Changed,
}

/// The local lights of a scene. See the [module documentation](self).
#[derive(Debug, Default)]
pub(crate) struct LightSet {
    lights: Vec<SceneLight>,
    runtime: Vec<LightRuntime>,
    links: Vec<Link>,
    /// By [`Id::index`]: the light's position in `lights`, or [`NONE`].
    index: Vec<u32>,
    /// By [`Id::index`] of a part: how many lights follow it.
    hosted: Vec<u32>,
    /// By [`Id::index`] of a part: the sync its transform last changed in.
    host_moved: Vec<u32>,
    /// The last sync in which a part holding lights moved.
    moved_in: Option<u32>,
    /// Changes whenever any light changed.
    revision: u64,
}

impl LightSet {
    pub(crate) fn len(&self) -> usize {
        self.lights.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    pub(crate) fn lights(&self) -> &[SceneLight] {
        &self.lights
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The lights and the renderer's state of each, side by side.
    pub(crate) fn split_mut(&mut self) -> (&[SceneLight], &mut [LightRuntime]) {
        (&self.lights, &mut self.runtime)
    }

    /// Forgets every light (a rebuild queues them all again). Keeps the memory.
    pub(crate) fn clear(&mut self) {
        self.lights.clear();
        self.runtime.clear();
        self.links.clear();
        self.index.fill(NONE);
        self.hosted.fill(0);
        self.moved_in = None;
        self.revision = self.revision.wrapping_add(1);
    }

    fn position_of(&self, id: Id) -> Option<usize> {
        let at = *self.index.get(id.index() as usize)?;
        (at != NONE && self.lights.get(at as usize).is_some_and(|l| l.id == id)).then_some(at as usize)
    }

    /// Removes `id` if it is a light here. Returns whether it was.
    pub(crate) fn remove(&mut self, id: Id) -> bool {
        let Some(at) = self.position_of(id) else {
            return false;
        };
        if let Some(host) = self.lights[at].host {
            self.unhost(host);
        }
        self.lights.swap_remove(at);
        self.runtime.swap_remove(at);
        self.links.swap_remove(at);
        if let Some(slot) = self.index.get_mut(id.index() as usize) {
            *slot = NONE;
        }
        if let Some(moved) = self.lights.get(at)
            && let Some(slot) = self.index.get_mut(moved.id.index() as usize)
        {
            *slot = at as u32;
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Applies a change to the light `id` of class `binding`. A change that names its place in the tree (or its
    /// creation) decides again whether it is in the world (a descendant of `world`, or anywhere without one) and
    /// which part it follows.
    pub(crate) fn apply(
        &mut self,
        dm: &DataModel,
        id: Id,
        bits: DirtyBits,
        binding: &LightBinding,
        world: Option<Id>,
        classes: &ClassTable,
    ) -> LightChange {
        let current = self.position_of(id);
        let structure = bits.intersects(DirtyBits::CREATED | DirtyBits::STRUCTURE);
        let placed = if structure {
            let in_world = match world {
                Some(world) => dm.is_descendant_of(id, world).unwrap_or(false),
                None => true,
            };
            if !in_world {
                return if self.remove(id) {
                    LightChange::Changed
                } else {
                    LightChange::None
                };
            }
            let host = dm
                .parent(id)
                .ok()
                .flatten()
                .and_then(|parent| Some((parent, classes.get(dm.class_of(parent).ok()?)?.props.cframe)));
            (
                host.map(|(part, _)| part),
                Link {
                    cframe: binding.props.cframe,
                    host_cframe: host.map(|(_, prop)| prop),
                },
            )
        } else {
            let Some(at) = current else {
                return LightChange::None;
            };
            (self.lights[at].host, self.links[at])
        };
        let (host, link) = placed;
        let light = read(dm, id, binding, host, link);
        match current {
            Some(at) => {
                let old = self.lights[at].host;
                if old != host {
                    if let Some(old) = old {
                        self.unhost(old);
                    }
                    if let Some(new) = host {
                        self.add_host(new);
                    }
                }
                self.lights[at] = light;
                self.links[at] = link;
            }
            None => {
                let index = id.index() as usize;
                if index >= self.index.len() {
                    // Grows only when the data model has grown: never in a steady frame.
                    self.index.resize(index + 1, NONE);
                }
                self.index[index] = self.lights.len() as u32;
                self.lights.push(light);
                self.runtime.push(LightRuntime::default());
                self.links.push(link);
                if let Some(host) = host {
                    self.add_host(host);
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
        LightChange::Changed
    }

    /// Notes that part `part`'s transform changed in sync `frame`, if lights follow it.
    #[inline]
    pub(crate) fn part_moved(&mut self, part: Id, frame: u32) {
        let index = part.index() as usize;
        if self.hosted.get(index).is_some_and(|&n| n > 0) {
            if let Some(stamp) = self.host_moved.get_mut(index) {
                *stamp = frame;
            }
            self.moved_in = Some(frame);
        }
    }

    /// Places again every light whose part moved in sync `frame`. Returns how many.
    pub(crate) fn place_moved(&mut self, dm: &DataModel, frame: u32) -> u32 {
        if self.moved_in != Some(frame) {
            return 0;
        }
        let mut placed = 0;
        for (light, link) in self.lights.iter_mut().zip(&self.links) {
            let Some(host) = light.host else { continue };
            if self.host_moved.get(host.index() as usize) != Some(&frame) {
                continue;
            }
            let (position, direction) = placement(dm, light.id, Some(host), *link);
            light.position = position;
            light.direction = direction;
            placed += 1;
        }
        if placed > 0 {
            self.revision = self.revision.wrapping_add(1);
        }
        placed
    }

    fn add_host(&mut self, part: Id) {
        let index = part.index() as usize;
        if index >= self.hosted.len() {
            self.hosted.resize(index + 1, 0);
            self.host_moved.resize(index + 1, 0);
        }
        self.hosted[index] += 1;
    }

    fn unhost(&mut self, part: Id) {
        if let Some(n) = self.hosted.get_mut(part.index() as usize) {
            *n = n.saturating_sub(1);
        }
    }
}

/// The light's world position and forward direction.
fn placement(dm: &DataModel, id: Id, host: Option<Id>, link: Link) -> (Vec3, Vec3) {
    let local = dm.cframe(id, link.cframe).unwrap_or(CFrame::IDENTITY);
    let world = match (host, link.host_cframe) {
        (Some(part), Some(prop)) => dm.cframe(part, prop).unwrap_or(CFrame::IDENTITY) * local,
        _ => local,
    };
    let direction = (world.rotation() * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
    (world.position(), direction)
}

/// The light `id` as its properties say now. Properties that cannot be read light with `Light`'s defaults.
fn read(dm: &DataModel, id: Id, binding: &LightBinding, host: Option<Id>, link: Link) -> SceneLight {
    let props = &binding.props;
    let number =
        |prop: Option<PropId>, default: f32| prop.and_then(|p| dm.number(id, p).ok()).map_or(default, |v| v as f32);
    let flag = |prop: Option<PropId>, default: bool| prop.and_then(|p| dm.bool(id, p).ok()).unwrap_or(default);
    let (position, direction) = placement(dm, id, host, link);
    let color = props.color.and_then(|p| dm.color3(id, p).ok()).unwrap_or(Color3::WHITE);
    let brightness = number(props.brightness, 1.0).max(0.0);
    SceneLight {
        id,
        kind: binding.kind,
        position,
        direction,
        color,
        brightness,
        intensity: color.to_linear().max(Vec3::ZERO) * brightness,
        range: number(props.range, 16.0).max(0.0),
        angle: match binding.kind {
            LightKind::Spot => number(props.angle, 90.0).clamp(1.0, 179.0),
            LightKind::Point => 0.0,
        },
        enabled: flag(props.enabled, true),
        shadows: flag(props.shadows, false),
        host,
    }
}
