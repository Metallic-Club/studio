//! Converting effects between art styles.
//!
//! [`Effect::restyle`] rewrites an effect for another [`ArtStyle`] without touching its motion design
//! (spawn timing, forces, lifetimes stay), so one preset library serves every game look. The rules
//! (assistant's call, from the stylised-VFX research in `docs/agents/41-vfx.md`):
//!
//! | Target | Textures | Colour and alpha | Motion |
//! |---|---|---|---|
//! | Realistic | soft variants | smooth gradients and fades | every frame |
//! | Toon | hard shapes, cel puffs | banded gradients (3 steps), cut-out alpha, fades become erosion | every frame |
//! | Pixel | 8x8 / 16x16 pixel sprites, nearest filtering | 4-step gradients and sizes, 3 alpha levels, 3 px snapping | stepped at 12 fps |
//! | Low-poly | shards and faceted hexagons | 4-step gradients, cut-out alpha, fades become erosion, tumbling | every frame |
//! | 2D | unchanged | unchanged | locked to the XY plane, ground-facing quads face the camera |

use crate::curve::Range;
use crate::curve::{ColorKey, Curve, Gradient, Interp, Key};
use crate::effect::{ArtStyle, Effect, Facing, InitModule, Texture, UpdateModule};

impl Effect {
    /// A copy of this effect converted to `target`. Converting to the effect's own style returns an
    /// unchanged copy.
    #[must_use]
    pub fn restyle(&self, target: ArtStyle) -> Effect {
        let mut e = self.clone();
        if target == self.style {
            return e;
        }
        e.style = target;
        for em in &mut e.emitters {
            em.render.texture = texture_for(&em.render.texture, target);
            if em.render.texture.sheet().is_none() && self_has_sheet(&em.render.flipbook) {
                em.render.flipbook = None;
            }
            match target {
                ArtStyle::Realistic | ArtStyle::Flat2D => {
                    if target == ArtStyle::Realistic {
                        em.render.alpha_steps = 0;
                        em.render.pixel_snap = 0.0;
                        em.fixed_fps = 0.0;
                    }
                }
                ArtStyle::Toon => {
                    em.render.alpha_steps = 1;
                    em.render.pixel_snap = 0.0;
                    em.fixed_fps = 0.0;
                    fades_to_erosion(&mut em.update);
                    band_gradients(&mut em.update, 3);
                }
                ArtStyle::Pixel => {
                    em.render.alpha_steps = 2;
                    em.render.pixel_snap = 3.0;
                    em.fixed_fps = 12.0;
                    band_gradients(&mut em.update, 4);
                    for m in &mut em.update {
                        if let UpdateModule::SizeOverLife { curve, .. } = m {
                            *curve = quantize_curve(curve, 4);
                        }
                    }
                }
                ArtStyle::LowPoly => {
                    em.render.alpha_steps = 1;
                    em.render.pixel_snap = 0.0;
                    em.fixed_fps = 0.0;
                    fades_to_erosion(&mut em.update);
                    band_gradients(&mut em.update, 4);
                    if !em
                        .init
                        .iter()
                        .any(|m| matches!(m, InitModule::Rotation { .. }))
                    {
                        em.init.push(InitModule::Rotation {
                            angle: Range::new(0.0, 360.0),
                            spin: Range::new(-120.0, 120.0),
                            disabled: false,
                        });
                    }
                }
            }
            if target == ArtStyle::Flat2D
                && matches!(
                    em.render.facing,
                    Facing::Ground | Facing::VelocityPerpendicular
                )
            {
                em.render.facing = Facing::Camera;
            }
        }
        for t in &mut e.trails {
            match target {
                ArtStyle::Toon | ArtStyle::LowPoly => t.color = quantize_gradient(&t.color, 3),
                ArtStyle::Pixel => {
                    t.color = quantize_gradient(&t.color, 4);
                    t.width_scale = quantize_curve(&t.width_scale, 4);
                }
                _ => {}
            }
        }
        for b in &mut e.beams {
            match target {
                ArtStyle::Toon | ArtStyle::LowPoly => b.color = quantize_gradient(&b.color, 3),
                ArtStyle::Pixel => {
                    b.color = quantize_gradient(&b.color, 4);
                    b.segments = b.segments.min(8);
                    if b.jitter_fps > 0.0 {
                        b.jitter_fps = b.jitter_fps.min(12.0);
                    }
                }
                _ => {}
            }
        }
        e
    }
}

fn self_has_sheet(fb: &Option<crate::effect::Flipbook>) -> bool {
    fb.as_ref().is_some_and(|f| f.frames() > 1)
}

/// The closest texture in the target style's family.
#[must_use]
pub fn texture_for(t: &Texture, style: ArtStyle) -> Texture {
    use Texture as T;
    match style {
        ArtStyle::Flat2D => t.clone(),
        ArtStyle::Realistic => match t {
            T::Dot | T::PixelDot => T::SoftDot,
            T::ToonPuff | T::PixelPuff | T::Hexagon => T::Smoke,
            T::PixelSpark | T::Shard => T::Spark,
            other => other.clone(),
        },
        ArtStyle::Toon => match t {
            T::SoftDot | T::PixelDot => T::Dot,
            T::Smoke | T::SmokeSheet | T::FlameSheet | T::PixelPuff | T::Hexagon => T::ToonPuff,
            T::PixelSpark => T::Twinkle,
            T::Shard => T::Spark,
            other => other.clone(),
        },
        ArtStyle::Pixel => match t {
            T::SoftDot | T::Dot | T::Bubble | T::Hexagon | T::Shard => T::PixelDot,
            T::Smoke | T::SmokeSheet | T::FlameSheet | T::ToonPuff => T::PixelPuff,
            T::Spark | T::Twinkle => T::PixelSpark,
            other => other.clone(),
        },
        ArtStyle::LowPoly => match t {
            T::SoftDot | T::Dot | T::PixelDot | T::Bubble => T::Hexagon,
            T::Smoke | T::SmokeSheet | T::FlameSheet | T::ToonPuff | T::PixelPuff => T::Hexagon,
            T::Spark | T::PixelSpark | T::Twinkle | T::Droplet => T::Shard,
            other => other.clone(),
        },
    }
}

/// Replaces transparency fades with erosion (shapes shrink and break up instead of going see-through).
fn fades_to_erosion(update: &mut [UpdateModule]) {
    for m in update.iter_mut() {
        if let UpdateModule::TransparencyOverLife { curve, disabled } = m {
            *m = UpdateModule::ErosionOverLife {
                curve: curve.clone(),
                disabled: *disabled,
            };
        }
    }
}

fn band_gradients(update: &mut [UpdateModule], steps: usize) {
    for m in update.iter_mut() {
        if let UpdateModule::ColorOverLife { gradient, .. } = m {
            *gradient = quantize_gradient(gradient, steps);
        }
    }
}

/// Resamples a curve into `steps` held values (sampled at each band's middle).
#[must_use]
pub fn quantize_curve(c: &Curve, steps: usize) -> Curve {
    let n = steps.max(1);
    if c.keys.len() <= 1 {
        return c.clone();
    }
    let keys = (0..n)
        .map(|i| {
            let t0 = i as f32 / n as f32;
            let mid = (i as f32 + 0.5) / n as f32;
            Key {
                t: t0,
                v: c.sample(mid),
                env: 0.0,
            }
        })
        .collect();
    Curve {
        keys,
        interp: Interp::Step,
    }
}

/// Resamples a gradient into `steps` flat colour bands.
#[must_use]
pub fn quantize_gradient(g: &Gradient, steps: usize) -> Gradient {
    let n = steps.max(1);
    if g.keys.len() <= 1 {
        return g.clone();
    }
    let keys = (0..n)
        .map(|i| ColorKey {
            t: i as f32 / n as f32,
            // Stored at 8 bits so the result survives a save and load unchanged.
            c: {
                let [r, gg, b] = g.sample((i as f32 + 0.5) / n as f32).to_u8();
                crate::color::Rgb::from_u8(r, gg, b)
            },
        })
        .collect();
    Gradient {
        keys,
        space: g.space,
        interp: Interp::Step,
    }
}
