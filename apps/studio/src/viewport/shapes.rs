//! Where a ray meets a part's actual shape (#494), for picking: a `Part`'s Block, Ball, Cylinder or Wedge stretched
//! to its `Size`, and a `MeshPart`'s triangles. Hover, clicks and Alt+clicks used to meet every part as its box, so
//! the empty corners round a ball picked it.
//!
//! The ray is taken into the part's unit cube (−0.5 to 0.5 on every axis, the space the built-in meshes and every
//! mesh asset are fitted to), where each shape has a simple test; a point `o + t·d` there is the same point, at the
//! same `t`, as along the world ray, so distances need no converting back.
//!
//! A `MeshPart`'s triangles are read from its project on a worker thread the first time a ray meets its box
//! ([`register_meshes`]), and kept for the data model they were read for; until they are back, or where they cannot
//! be read, the part is met as its box.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{DataModel, Id, Value};
use metallic_engine::metallic_project::AssetReader;

use super::pick::{Bounds, is_a};
use super::view::Ray;

/// A mesh's triangles, in the unit cube.
pub type Triangles = Arc<[[Vec3; 3]]>;

/// A part's shape, as picking meets it.
#[derive(Clone, Debug)]
pub enum Shape {
    /// The whole box.
    Block,
    /// The ellipsoid inside the box.
    Ball,
    /// The cylinder along the box's X axis, its ends the ellipse of its Y and Z sides.
    Cylinder,
    /// The ramp from the top of the +Z side down to the bottom of the −Z side.
    Wedge,
    /// A mesh asset's triangles, in the unit cube.
    Mesh(Triangles),
}

/// The shape of part `id` in `dm`: its `Shape` for a `Part`, its mesh for a `MeshPart` whose triangles are here,
/// otherwise its box.
pub fn shape_of(dm: &DataModel, id: Id) -> Shape {
    if is_a(dm, id, "MeshPart") {
        return match dm.get_by_name(id, "MeshId") {
            Ok(Value::String(mesh)) if !mesh.is_empty() => {
                mesh_triangles(dm.uid(), &mesh).map_or(Shape::Block, Shape::Mesh)
            }
            _ => Shape::Block,
        };
    }
    match dm.get_by_name(id, "Shape") {
        Ok(Value::Enum(item)) => match item.name() {
            "Ball" => Shape::Ball,
            "Cylinder" => Shape::Cylinder,
            "Wedge" => Shape::Wedge,
            _ => Shape::Block,
        },
        _ => Shape::Block,
    }
}

/// Where `ray` first meets `shape` stretched to `bounds` (how far along it), if it does: 0 from inside a solid shape.
pub fn hit(bounds: &Bounds, shape: &Shape, (origin, dir): Ray) -> Option<f32> {
    let inverse = bounds.rot.inverse();
    let size = (bounds.half * 2.0).max(Vec3::splat(1e-6));
    let o = inverse * (origin - bounds.center) / size;
    let d = inverse * dir / size;
    let (near, far) = unit_box(o, d)?;
    match shape {
        Shape::Block => Some(near.max(0.0)),
        Shape::Ball => ball(o, d),
        Shape::Cylinder => cylinder(o, d, (near, far)),
        Shape::Wedge => wedge(o, d, (near, far)),
        Shape::Mesh(triangles) => mesh(o, d, triangles),
    }
}

/// The span of `t` over which `o + t·d` is inside the unit cube, if it ever is and not only behind the start.
fn unit_box(o: Vec3, d: Vec3) -> Option<(f32, f32)> {
    let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
    for axis in 0..3 {
        if d[axis].abs() < 1e-12 {
            if o[axis].abs() > 0.5 {
                return None;
            }
            continue;
        }
        let a = (-0.5 - o[axis]) / d[axis];
        let b = (0.5 - o[axis]) / d[axis];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far >= 0.0).then_some((near, far))
}

/// The roots of `a·t² + 2b·t + c = 0`, the smaller first.
fn roots(a: f32, b: f32, c: f32) -> Option<(f32, f32)> {
    let disc = b * b - a * c;
    if disc < 0.0 || a.abs() < 1e-12 {
        return None;
    }
    let root = disc.sqrt();
    Some(((-b - root) / a, (-b + root) / a))
}

/// The ball of radius 0.5.
fn ball(o: Vec3, d: Vec3) -> Option<f32> {
    let (t0, t1) = roots(d.dot(d), o.dot(d), o.dot(o) - 0.25)?;
    (t1 >= 0.0).then(|| t0.max(0.0))
}

/// The cylinder of radius 0.5 along X, cut by the cube's X sides (`span`).
fn cylinder(o: Vec3, d: Vec3, (near, far): (f32, f32)) -> Option<f32> {
    let a = d.y * d.y + d.z * d.z;
    let c = o.y * o.y + o.z * o.z - 0.25;
    let (t0, t1) = if a < 1e-12 {
        // Along the axis: inside the round side all the way, or never.
        if c > 0.0 {
            return None;
        }
        (f32::NEG_INFINITY, f32::INFINITY)
    } else {
        roots(a, o.y * d.y + o.z * d.z, c)?
    };
    let (near, far) = (near.max(t0), far.min(t1));
    (near <= far && far >= 0.0).then(|| near.max(0.0))
}

/// The cube below the plane `y = z`: the ramp, high at +Z.
fn wedge(o: Vec3, d: Vec3, (mut near, mut far): (f32, f32)) -> Option<f32> {
    let g = o.y - o.z;
    let k = d.y - d.z;
    if k.abs() < 1e-12 {
        if g > 0.0 {
            return None;
        }
    } else {
        let t = -g / k;
        if k > 0.0 {
            far = far.min(t);
        } else {
            near = near.max(t);
        }
    }
    (near <= far && far >= 0.0).then(|| near.max(0.0))
}

/// The nearest triangle `o + t·d` meets ahead (Möller–Trumbore, either face).
fn mesh(o: Vec3, d: Vec3, triangles: &[[Vec3; 3]]) -> Option<f32> {
    let mut nearest: Option<f32> = None;
    for [a, b, c] in triangles {
        let (e1, e2) = (*b - *a, *c - *a);
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = o - *a;
        let u = s.dot(p) / det;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = e2.dot(q) / det;
        if t >= 0.0 && nearest.is_none_or(|n| t < n) {
            nearest = Some(t);
        }
    }
    nearest
}

/// Reads the `MeshPart` meshes of the data model numbered `dm` ([`DataModel::uid`]) with `reader` from now on, on a
/// worker thread. A data model registered again (a project opened again) starts over.
pub fn register_meshes(dm: u64, reader: AssetReader) {
    let (jobs, inbox) = mpsc::channel::<Arc<str>>();
    let (results, loaded) = mpsc::channel();
    let mut reader = reader;
    let worker = std::thread::Builder::new()
        .name("metallic pick meshes".into())
        .spawn(move || {
            while let Ok(id) = inbox.recv() {
                let triangles = reader.load_mesh(&id).ok().map(|mesh| triangles_of(&mesh));
                if results.send((id, triangles)).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = worker {
        log::warn!("pick: meshes are picked by their boxes ({error})");
        return;
    }
    let mut sources = sources().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sources.insert(
        dm,
        Source {
            jobs,
            loaded,
            meshes: HashMap::new(),
        },
    );
}

/// A mesh asset's triangles, in the unit cube.
fn triangles_of(mesh: &metallic_engine::metallic_project::MeshAsset) -> Triangles {
    let at = |i: u32| {
        mesh.positions
            .get(i as usize)
            .map_or(Vec3::ZERO, |&p| Vec3::from_array(p))
    };
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| [at(t[0]), at(t[1]), at(t[2])])
        .collect()
}

/// One data model's meshes: those back, and the worker reading the rest.
struct Source {
    jobs: Sender<Arc<str>>,
    loaded: Receiver<(Arc<str>, Option<Triangles>)>,
    /// Every mesh asked for: `None` while it is read, or when it could not be.
    meshes: HashMap<Arc<str>, Option<Triangles>>,
}

fn sources() -> &'static Mutex<HashMap<u64, Source>> {
    static SOURCES: OnceLock<Mutex<HashMap<u64, Source>>> = OnceLock::new();
    SOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mesh `id`'s triangles for data model `dm`, if they are here; asks for them the first time.
fn mesh_triangles(dm: u64, id: &str) -> Option<Triangles> {
    let mut sources = sources().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let source = sources.get_mut(&dm)?;
    while let Ok((loaded, triangles)) = source.loaded.try_recv() {
        source.meshes.insert(loaded, triangles);
    }
    match source.meshes.get(id) {
        Some(triangles) => triangles.clone(),
        None => {
            let id: Arc<str> = id.into();
            if source.jobs.send(Arc::clone(&id)).is_ok() {
                source.meshes.insert(id, None);
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use metallic_engine::metallic_core::glam::Quat;

    use super::*;

    fn unit(size: Vec3) -> Bounds {
        Bounds {
            center: Vec3::ZERO,
            half: size * 0.5,
            rot: Quat::IDENTITY,
        }
    }

    /// A ray from far along +Z towards −Z, through `(x, y)`.
    fn down_z(x: f32, y: f32) -> Ray {
        (Vec3::new(x, y, 50.0), Vec3::NEG_Z)
    }

    #[test]
    fn a_ball_is_met_as_a_ball_not_its_box() {
        let bounds = unit(Vec3::splat(4.0));
        // Through the middle: its front, 2 m before the centre.
        let t = hit(&bounds, &Shape::Ball, down_z(0.0, 0.0)).unwrap();
        assert!((t - 48.0).abs() < 1e-3, "{t}");
        // Through a corner of the box, outside the ball: nothing, where the box is met.
        assert!(hit(&bounds, &Shape::Ball, down_z(1.8, 1.8)).is_none());
        assert!(hit(&bounds, &Shape::Block, down_z(1.8, 1.8)).is_some());
        // An ellipsoid: 4 wide, 2 high.
        let squashed = unit(Vec3::new(4.0, 2.0, 2.0));
        assert!(hit(&squashed, &Shape::Ball, down_z(1.9, 0.0)).is_some());
        assert!(hit(&squashed, &Shape::Ball, down_z(1.6, 0.8)).is_none());
    }

    #[test]
    fn a_cylinder_runs_along_x_and_a_wedge_slopes_to_minus_z() {
        let bounds = unit(Vec3::splat(2.0));
        // Down Z through the cylinder's round side: met near the axis, missed off it in Y.
        assert!(hit(&bounds, &Shape::Cylinder, down_z(0.9, 0.0)).is_some());
        assert!(hit(&bounds, &Shape::Cylinder, down_z(0.0, 0.95)).is_some());
        let (across, along_axis) = (
            (Vec3::new(0.0, 0.9, 50.0), Vec3::NEG_Z),
            (Vec3::new(50.0, 0.9, 0.9), Vec3::NEG_X),
        );
        let t = hit(&bounds, &Shape::Cylinder, across).unwrap();
        assert!(t > 49.0, "met on its round side, {t}");
        assert!(
            hit(&bounds, &Shape::Cylinder, along_axis).is_none(),
            "outside its round side"
        );
        // A wedge: from above, its top at +Z is high and its front at −Z low.
        let from_above = |z: f32| (Vec3::new(0.0, 50.0, z), Vec3::NEG_Y);
        let back = hit(&bounds, &Shape::Wedge, from_above(0.9)).unwrap();
        let front = hit(&bounds, &Shape::Wedge, from_above(-0.9)).unwrap();
        assert!(
            (back - 49.1).abs() < 1e-3 && (front - 50.9).abs() < 1e-3,
            "{back} {front}"
        );
        // From the front, high up: over the slope, where the box would be met.
        let high = (Vec3::new(0.0, 0.8, -50.0), Vec3::Z);
        let t = hit(&bounds, &Shape::Wedge, high).unwrap();
        assert!((t - 50.8).abs() < 1e-3, "{t}");
        let over = (Vec3::new(0.0, 1.5, -50.0), Vec3::Z);
        assert!(hit(&bounds, &Shape::Wedge, over).is_none());
    }

    #[test]
    fn a_mesh_is_met_by_its_triangles() {
        // One triangle in the unit cube's middle plane, filling its lower-left half.
        let triangle: Triangles = Arc::from(vec![[
            Vec3::new(-0.5, -0.5, 0.0),
            Vec3::new(0.5, -0.5, 0.0),
            Vec3::new(-0.5, 0.5, 0.0),
        ]]);
        let bounds = unit(Vec3::splat(2.0));
        let shape = Shape::Mesh(triangle);
        let t = hit(&bounds, &shape, down_z(-0.5, -0.5)).unwrap();
        assert!((t - 50.0).abs() < 1e-3, "{t}");
        assert!(hit(&bounds, &shape, down_z(0.6, 0.6)).is_none(), "the empty half");
        // A part whose mesh is not here is met as its box.
        let dm = metallic_engine::metallic_classes::new_game().unwrap();
        assert!(mesh_triangles(dm.uid(), "Meshes/none.glb").is_none());
    }

    #[test]
    fn a_turned_and_moved_shape_is_met_where_it_is() {
        let bounds = Bounds {
            center: Vec3::new(10.0, 0.0, 0.0),
            half: Vec3::new(2.0, 0.5, 0.5),
            rot: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        // The cylinder's axis turned from X to −Z: down Z it is met end on, 2 m before its centre.
        let t = hit(&bounds, &Shape::Cylinder, (Vec3::new(10.0, 0.0, 50.0), Vec3::NEG_Z)).unwrap();
        assert!((t - 48.0).abs() < 1e-3, "{t}");
        assert!(hit(&bounds, &Shape::Cylinder, (Vec3::new(10.0, 0.45, 50.0), Vec3::NEG_Z)).is_some());
        assert!(hit(&bounds, &Shape::Cylinder, (Vec3::new(10.45, 0.45, 50.0), Vec3::NEG_Z)).is_none());
    }
}
