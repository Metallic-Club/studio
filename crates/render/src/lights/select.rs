//! Choosing the lights a frame draws: candidates in view, ranked by importance, the budget, and the fades (see the
//! [module documentation](super)). CPU only.

use metallic_core::glam::Vec3;

use super::{BUDGET_HYSTERESIS, LightRuntime, LightSettings, MAX_GPU_LIGHTS};
use crate::cull::Frustum;
use crate::scene::{LightKind, SceneLight};

/// The sphere a light reaches, in world space: its range around it for a point light, the smallest sphere around
/// its cone (capped by its range) for a spot.
pub(crate) fn influence(light: &SceneLight) -> (Vec3, f32) {
    let range = light.range;
    match light.kind {
        LightKind::Point => (light.position, range),
        LightKind::Spot => {
            let half = (light.angle * 0.5).to_radians().clamp(0.0, std::f32::consts::FRAC_PI_2);
            let (sin, cos) = half.sin_cos();
            if half >= std::f32::consts::FRAC_PI_4 {
                // Wide: the sphere through the rim; it holds the apex and the cap's tip too.
                (light.position + light.direction * (range * cos), range * sin)
            } else {
                // Narrow: the sphere through the apex and the rim.
                let radius = range / (2.0 * cos.max(1e-3));
                (light.position + light.direction * radius, radius)
            }
        }
    }
}

/// How much a light matters to the frame seen from `eye`: brightness × the colour's luminance × how large its
/// sphere looks (radius over distance, 1 from inside).
pub(crate) fn importance(light: &SceneLight, center: Vec3, radius: f32, eye: Vec3) -> f32 {
    let luminance = light.intensity.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let distance = center.distance(eye);
    luminance * radius / distance.max(radius).max(1e-3)
}

/// What a selection did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) in_view: u32,
    pub(crate) in_budget: u32,
    pub(crate) drawn: u32,
    pub(crate) fading: u32,
}

/// The view a selection is made for: the camera's frustum (origin-relative, as culling builds it), the origin, the
/// camera's position and the view distance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectView<'a> {
    pub(crate) frustum: &'a Frustum,
    pub(crate) origin: Vec3,
    pub(crate) eye: Vec3,
    pub(crate) max_distance: f32,
}

/// Ranks the lights that shine into `view`, moves each one's fade towards its target by `dt` seconds, and writes
/// the lights to draw into `drawn` (indices into `lights`, most important first, at most [`MAX_GPU_LIGHTS`]).
/// `ranked` is scratch. Allocates nothing once the vectors have grown to the scene's lights.
pub(crate) fn select(
    lights: &[SceneLight],
    runtime: &mut [LightRuntime],
    view: &SelectView<'_>,
    settings: &LightSettings,
    dt: f32,
    ranked: &mut Vec<(f32, u32)>,
    drawn: &mut Vec<u32>,
) -> Selection {
    ranked.clear();
    drawn.clear();
    let budget = settings.light_budget();
    for (index, (light, state)) in (0u32..).zip(lights.iter().zip(runtime.iter_mut())) {
        state.drawn = u16::MAX;
        state.fresh = false;
        if budget == 0 || !light.shines() {
            *state = LightRuntime {
                tiles: state.tiles,
                tile_count: state.tile_count,
                drawn: u16::MAX,
                ..LightRuntime::default()
            };
            continue;
        }
        let (center, radius) = influence(light);
        let in_view = view.frustum.contains_sphere(center - view.origin, radius)
            && center.distance(view.eye) - radius <= view.max_distance;
        if !in_view {
            // Out of view it lights nothing on screen: it starts at its target when it comes back.
            state.seen = false;
            state.fade = 0.0;
            state.in_budget = false;
            state.shadow_fade = 0.0;
            continue;
        }
        let mut key = importance(light, center, radius, view.eye);
        if state.in_budget {
            key *= BUDGET_HYSTERESIS;
        }
        ranked.push((key, index));
    }
    // Most important first; ties by index, so the order does not depend on the sort.
    ranked.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let step = if settings.fade_time > 0.0 {
        (dt / settings.fade_time).max(0.0)
    } else {
        1.0
    };
    let mut selection = Selection {
        in_view: ranked.len() as u32,
        ..Selection::default()
    };
    for (rank, &(_, index)) in ranked.iter().enumerate() {
        let Some(state) = runtime.get_mut(index as usize) else {
            continue;
        };
        let target = rank < budget;
        state.in_budget = target;
        let goal = if target { 1.0 } else { 0.0 };
        if !state.seen {
            state.seen = true;
            state.fresh = true;
            state.fade = goal;
        } else if state.fade < goal {
            state.fade = (state.fade + step).min(goal);
        } else {
            state.fade = (state.fade - step).max(goal);
        }
        if target {
            selection.in_budget += 1;
        }
        if state.fade <= 0.0 {
            continue;
        }
        if drawn.len() >= MAX_GPU_LIGHTS {
            // Only lights fading out get here (the budget is at most the capacity): they stop now.
            state.fade = 0.0;
            continue;
        }
        if state.fade < 1.0 {
            selection.fading += 1;
        }
        state.drawn = drawn.len() as u16;
        drawn.push(index);
    }
    selection.drawn = drawn.len() as u32;
    selection
}

#[cfg(test)]
mod tests {
    use metallic_core::{Color3, Id};

    use super::*;

    fn light(x: f32, brightness: f32) -> SceneLight {
        SceneLight {
            id: Id::new(1, 0),
            kind: LightKind::Point,
            position: Vec3::new(x, 0.0, -10.0),
            direction: Vec3::NEG_Z,
            color: Color3::WHITE,
            brightness,
            intensity: Vec3::splat(brightness),
            range: 8.0,
            angle: 0.0,
            enabled: true,
            shadows: false,
            host: None,
        }
    }

    /// Looking down −Z from the origin, 90° wide.
    fn frustum() -> Frustum {
        let camera = crate::Camera::from_cframe(metallic_core::CFrame::IDENTITY, 90.0);
        Frustum::from_clip(camera.view_projection(1.0))
    }

    fn run(lights: &[SceneLight], runtime: &mut [LightRuntime], budget: u32, dt: f32) -> (Selection, Vec<u32>) {
        let frustum = frustum();
        let view = SelectView {
            frustum: &frustum,
            origin: Vec3::ZERO,
            eye: Vec3::ZERO,
            max_distance: f32::INFINITY,
        };
        let settings = LightSettings {
            budget,
            fade_time: 0.25,
            ..LightSettings::MEDIUM
        };
        let (mut ranked, mut drawn) = (Vec::new(), Vec::new());
        let selection = select(lights, runtime, &view, &settings, dt, &mut ranked, &mut drawn);
        (selection, drawn)
    }

    #[test]
    fn a_spot_cones_sphere_holds_its_cone() {
        for angle in [10.0f32, 60.0, 90.0, 120.0, 179.0] {
            let spot = SceneLight {
                kind: LightKind::Spot,
                angle,
                direction: Vec3::NEG_Y,
                ..light(0.0, 1.0)
            };
            let (center, radius) = influence(&spot);
            let half = (angle * 0.5).to_radians();
            for t in [0.0f32, 0.5, 1.0] {
                for side in [Vec3::X, Vec3::Z, Vec3::NEG_X] {
                    // Points on the cone's edge and its tip, up to the range.
                    let edge = spot.position + (Vec3::NEG_Y * half.cos() + side * half.sin()) * spot.range * t;
                    assert!(edge.distance(center) <= radius + 1e-3, "{angle}°: edge at {t}");
                }
                let tip = spot.position + Vec3::NEG_Y * spot.range * t;
                assert!(tip.distance(center) <= radius + 1e-3, "{angle}°: tip at {t}");
            }
            assert!(radius <= spot.range + 1e-3, "never larger than the point light's sphere");
        }
    }

    #[test]
    fn the_budget_keeps_the_most_important_and_the_rest_fade_out() {
        // Brighter and brighter from left to right; all in view.
        let lights: Vec<_> = (0..6).map(|i| light(i as f32 - 2.5, 1.0 + i as f32)).collect();
        let mut runtime = vec![LightRuntime::default(); lights.len()];
        let (selection, drawn) = run(&lights, &mut runtime, 3, 1.0 / 60.0);
        assert_eq!(selection.in_view, 6);
        assert_eq!(selection.in_budget, 3);
        // First sight: no fading, the three brightest at full strength.
        assert_eq!(drawn, [5, 4, 3]);
        assert_eq!(selection.fading, 0);
        assert!(runtime[5].fade == 1.0 && runtime[0].fade == 0.0);

        // The dimmest becomes the brightest: it fades in, and the one it pushes out fades out, both drawn meanwhile.
        let mut lights = lights;
        lights[0].intensity = Vec3::splat(20.0);
        let (selection, drawn) = run(&lights, &mut runtime, 3, 0.1);
        assert_eq!(selection.in_budget, 3);
        assert_eq!(drawn.len(), 4, "{drawn:?}");
        assert_eq!(selection.fading, 2);
        assert!((runtime[0].fade - 0.4).abs() < 1e-5, "{}", runtime[0].fade);
        assert!((runtime[3].fade - 0.6).abs() < 1e-5, "{}", runtime[3].fade);
        // Never a jump of more than one step per frame, and settled after the fade time.
        for _ in 0..3 {
            run(&lights, &mut runtime, 3, 0.1);
        }
        let (selection, drawn) = run(&lights, &mut runtime, 3, 0.1);
        assert_eq!((selection.fading, drawn.len()), (0, 3));
        assert_eq!((runtime[0].fade, runtime[3].fade), (1.0, 0.0));
    }

    #[test]
    fn lights_near_the_edge_of_the_budget_do_not_trade_places() {
        let mut lights = vec![light(-1.0, 2.0), light(1.0, 2.0)];
        let mut runtime = vec![LightRuntime::default(); 2];
        let (_, drawn) = run(&lights, &mut runtime, 1, 0.1);
        assert_eq!(drawn, [0]);
        // The other one gets a little brighter, within the hysteresis: nothing changes.
        lights[1].intensity = Vec3::splat(2.2);
        let (_, drawn) = run(&lights, &mut runtime, 1, 0.1);
        assert_eq!(drawn, [0]);
        // Much brighter: it takes the place.
        lights[1].intensity = Vec3::splat(4.0);
        run(&lights, &mut runtime, 1, 0.1);
        assert!(runtime[1].in_budget && !runtime[0].in_budget);
    }

    #[test]
    fn lights_out_of_view_or_off_are_not_candidates() {
        let mut behind = light(0.0, 1.0);
        behind.position = Vec3::new(0.0, 0.0, 20.0);
        let mut off = light(0.0, 1.0);
        off.enabled = false;
        let mut black = light(0.0, 1.0);
        black.intensity = Vec3::ZERO;
        // Behind the camera but reaching in front of it: a candidate.
        let mut reaching = light(0.0, 1.0);
        reaching.position = Vec3::new(0.0, 0.0, 4.0);
        let lights = [behind, off, black, reaching];
        let mut runtime = vec![LightRuntime::default(); lights.len()];
        let (selection, drawn) = run(&lights, &mut runtime, 8, 0.1);
        assert_eq!(selection.in_view, 1);
        assert_eq!(drawn, [3]);
    }
}
