//! `DirectionalLight` instances as extra suns (#123): which of them a frame draws, and what the frame uniform holds of
//! them (see the [module documentation](super)). CPU only, and out of line: the parts' sync and cull never meet it.

use metallic_core::glam::Vec3;

use super::MAX_EXTRA_SUNS;
use crate::lighting::SUN_RADIANCE;
use crate::scene::SceneLight;

/// The radiance of a `DirectionalLight` per unit of `Brightness`, linear: the sun's ([`SUN_RADIANCE`]), so a white
/// `DirectionalLight` of `Brightness` 2 lights like `Lighting`'s noon sun at its default `Brightness`.
pub const DIRECTIONAL_RADIANCE: f32 = SUN_RADIANCE;

/// The luminance weights of linear Rec. 709 colours, which rank lights of different colours.
const LUMINANCE: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// What the parts' frame uniform holds of the extra suns (the `extra_sun*` members of `Frame` in frame.wgsl).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameSuns {
    /// x: how many are drawn (0: none, and the shader's loop over them runs no iteration).
    pub(crate) count: [f32; 4],
    /// Per light: xyz, the unit vector towards it (against its rays).
    pub(crate) directions: [[f32; 4]; MAX_EXTRA_SUNS],
    /// Per light: rgb, its radiance.
    pub(crate) colors: [[f32; 4]; MAX_EXTRA_SUNS],
}

impl FrameSuns {
    /// How many are drawn.
    pub(crate) fn len(&self) -> u32 {
        self.count[0] as u32
    }
}

/// How bright a directional light is for the budget: its `Brightness` × its colour's luminance (both in its
/// intensity).
fn importance(light: &SceneLight) -> f32 {
    light.intensity.dot(LUMINANCE)
}

/// The extra suns a frame draws: of `lights` (the scene's directional lights), the `budget` brightest that shine (at
/// most [`MAX_EXTRA_SUNS`]), ranked by [`importance`], ties to the lower instance index. Returns them and how many
/// shine. Allocates nothing.
///
/// Nothing fades: which lights are in the budget depends on the lights alone, never on the camera, so it changes only
/// when the game changes a light (or the preset changes), and what the game does is drawn at once.
#[inline(never)]
pub(crate) fn frame_suns(lights: &[SceneLight], budget: u32) -> (FrameSuns, u32) {
    let budget = (budget as usize).min(MAX_EXTRA_SUNS);
    let mut chosen: [(f32, u32, usize); MAX_EXTRA_SUNS] = [(0.0, 0, 0); MAX_EXTRA_SUNS];
    let mut count = 0;
    let mut shining = 0;
    for (at, light) in lights.iter().enumerate() {
        if !light.shines() {
            continue;
        }
        shining += 1;
        if budget == 0 {
            continue;
        }
        let key = (importance(light), light.id.index(), at);
        let before = |a: &(f32, u32, usize), b: &(f32, u32, usize)| a.0 > b.0 || (a.0 == b.0 && a.1 < b.1);
        // Insertion into the few kept, brightest first.
        let mut place = count;
        while place > 0 && before(&key, &chosen[place - 1]) {
            place -= 1;
        }
        if place >= budget {
            continue;
        }
        let end = count.min(budget - 1);
        chosen.copy_within(place..end, place + 1);
        chosen[place] = key;
        count = (count + 1).min(budget);
    }
    let mut suns = FrameSuns {
        count: [count as f32, 0.0, 0.0, 0.0],
        ..FrameSuns::default()
    };
    for (i, &(_, _, at)) in chosen[..count].iter().enumerate() {
        let light = &lights[at];
        // The shader wants the direction towards the light: against its rays, which go along its forward direction.
        let towards = (-light.direction).normalize_or(Vec3::Y);
        let radiance = light.intensity * DIRECTIONAL_RADIANCE;
        suns.directions[i] = [towards.x, towards.y, towards.z, 0.0];
        suns.colors[i] = [radiance.x, radiance.y, radiance.z, 0.0];
    }
    (suns, shining)
}

#[cfg(test)]
mod tests {
    use metallic_core::{Color3, Id};

    use super::*;
    use crate::scene::LightKind;

    fn sun(index: u32, brightness: f32, color: Color3, direction: Vec3) -> SceneLight {
        SceneLight {
            id: Id::new(index, 1),
            kind: LightKind::Directional,
            position: Vec3::ZERO,
            direction,
            color,
            brightness,
            intensity: color.to_linear() * brightness,
            range: 0.0,
            angle: 0.0,
            enabled: true,
            shadows: false,
            host: None,
        }
    }

    #[test]
    fn the_brightest_shining_lights_are_drawn_from_their_forward_direction() {
        let down = Vec3::new(0.0, -1.0, 0.0);
        let mut off = sun(9, 10.0, Color3::WHITE, down);
        off.enabled = false;
        let lights = [
            sun(1, 1.0, Color3::WHITE, down),
            // Bright but blue: blue counts for little luminance.
            sun(2, 3.0, Color3::new(0.0, 0.0, 1.0), down),
            sun(3, 2.0, Color3::WHITE, Vec3::new(1.0, 0.0, 0.0)),
            off,
            sun(4, 0.0, Color3::WHITE, down),
        ];
        let (suns, shining) = frame_suns(&lights, 2);
        assert_eq!((suns.len(), shining), (2, 3));
        // The brightest first: light 3, whose rays go along +X, so it is towards −X.
        assert_eq!(suns.directions[0], [-1.0, 0.0, 0.0, 0.0]);
        assert_eq!(suns.directions[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(
            suns.colors[0],
            [
                2.0 * DIRECTIONAL_RADIANCE,
                2.0 * DIRECTIONAL_RADIANCE,
                2.0 * DIRECTIONAL_RADIANCE,
                0.0
            ]
        );
        // A budget of 0 draws none, one of 4 every one that shines, and never more than fit.
        assert_eq!(frame_suns(&lights, 0).0, FrameSuns::default());
        assert_eq!(frame_suns(&lights, 4).0.len(), 3);
        let many: Vec<SceneLight> = (0..10).map(|i| sun(i, 1.0 + i as f32, Color3::WHITE, down)).collect();
        let (suns, shining) = frame_suns(&many, 100);
        assert_eq!((suns.len(), shining), (MAX_EXTRA_SUNS as u32, 10));
        let brightest: Vec<f32> = (0..MAX_EXTRA_SUNS)
            .map(|i| (suns.colors[i][0] / DIRECTIONAL_RADIANCE).round())
            .collect();
        assert_eq!(brightest, [10.0, 9.0, 8.0, 7.0]);
    }

    #[test]
    fn equal_lights_rank_by_instance_whatever_their_order() {
        let down = Vec3::NEG_Y;
        let a = sun(5, 1.0, Color3::WHITE, Vec3::NEG_X);
        let b = sun(3, 1.0, Color3::WHITE, down);
        let one = frame_suns(&[a, b], 1).0;
        let other = frame_suns(&[b, a], 1).0;
        assert_eq!(one, other);
        assert_eq!(one.directions[0], [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn nonsense_directions_stay_finite() {
        let (suns, _) = frame_suns(&[sun(1, 1.0, Color3::WHITE, Vec3::ZERO)], 1);
        assert!(suns.directions[0].iter().all(|v| v.is_finite()));
    }
}
