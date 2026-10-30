//! The sky and sun: a vertical gradient (also the ambient light), one directional sun and fog.
use glam::Vec3;

/// Colours and light of the environment. Colours are linear RGB.
///
/// The defaults follow the Metallic brand: graphite ground, silver horizon, deep violet-graphite zenith.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Colour straight up.
    pub zenith: [f32; 3],
    /// Colour at the horizon; distant objects fade into it.
    pub horizon: [f32; 3],
    /// Colour below the horizon, also the ambient light coming from the ground.
    pub ground: [f32; 3],
    /// Unit vector pointing from the scene toward the sun.
    pub to_sun: Vec3,
    /// Sun radiance, linear RGB.
    pub sun_color: [f32; 3],
    /// Distance fog strength (1 / distance in studs where fog reaches about 63%).
    pub fog_density: f32,
    /// Exposure multiplier applied before tone mapping.
    pub exposure: f32,
}

impl Default for Sky {
    fn default() -> Self {
        Sky {
            zenith: [0.05, 0.058, 0.17],
            horizon: [0.30, 0.315, 0.41],
            ground: [0.085, 0.085, 0.10],
            to_sun: Vec3::new(0.5, 0.55, 0.35).normalize(),
            sun_color: [3.5, 3.3, 3.05],
            fog_density: 1.0 / 900.0,
            exposure: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sun_points_up_and_is_unit_length() {
        let s = Sky::default();
        assert!((s.to_sun.length() - 1.0).abs() < 1e-5);
        assert!(s.to_sun.y > 0.3);
    }
}
