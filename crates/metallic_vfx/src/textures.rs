//! Procedural built-in particle textures.
//!
//! Every [`Texture`] except `Asset` is generated here from a few lines of maths, so effects need no
//! downloads and look the same in the editor, in tests and (once uploaded) on the GPU. Textures are
//! white or grey with straight alpha; the particle colour tints them. Pixel-art textures are tiny and must
//! be sampled with nearest filtering ([`Image::nearest`]).

use crate::effect::Texture;
use crate::math::{Vec3, smoothstep};
use crate::noise::fbm3_grad;

/// An RGBA8 image with straight (non-premultiplied) alpha, rows top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Sample with nearest filtering (pixel art).
    pub nearest: bool,
}

/// Generates a built-in texture. `Asset` textures return the soft dot as a placeholder.
#[must_use]
pub fn generate(texture: &Texture) -> Image {
    match texture {
        Texture::SoftDot | Texture::Asset(_) => field(64, |x, y| {
            let r = (x * x + y * y).sqrt();
            let a = (-r * r * 3.2).exp() * (1.0 - smoothstep(0.8, 1.0, r));
            (1.0, a)
        }),
        Texture::Dot => field(64, |x, y| {
            let r = (x * x + y * y).sqrt();
            (1.0, 1.0 - smoothstep(0.86, 0.92, r))
        }),
        Texture::Spark => field(64, |x, y| {
            let core = (-(x / 0.16).powi(2)).exp() * (-(y / 0.75).powi(2)).exp();
            let hot = (-(x * x + y * y) / 0.04).exp();
            (1.0, (core + hot).min(1.0) * (1.0 - smoothstep(0.9, 1.0, y.abs())))
        }),
        Texture::Twinkle => field(64, |x, y| {
            let r2 = x * x + y * y;
            let arm = |a: f32, b: f32| (-(a / 0.07).powi(2)).exp() * (1.0 - b.abs()).max(0.0).powf(1.5);
            let a = arm(x, y).max(arm(y, x)) + (-r2 / 0.05).exp() * 0.8;
            (1.0, a.min(1.0))
        }),
        Texture::Ring => field(64, |x, y| {
            let r = (x * x + y * y).sqrt();
            (1.0, (-((r - 0.78) / 0.09).powi(2)).exp())
        }),
        Texture::Smoke => field(64, |x, y| {
            let r = (x * x + y * y).sqrt();
            let n = fbm(x * 2.2, y * 2.2, 3.0, 4);
            let mask = 1.0 - smoothstep(0.35, 1.0, r + n * 0.35);
            let shade = 0.78 + 0.22 * (0.5 - 0.5 * (x - y) * 0.7) + n * 0.15;
            (shade.clamp(0.0, 1.0), mask * 0.9)
        }),
        Texture::ToonPuff => field(96, |x, y| {
            let blob = |cx: f32, cy: f32, r: f32| r - ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let d = blob(0.0, -0.1, 0.62)
                .max(blob(-0.42, -0.25, 0.42))
                .max(blob(0.42, -0.22, 0.45))
                .max(blob(-0.12, 0.33, 0.45))
                .max(blob(0.3, 0.22, 0.38));
            let inside = smoothstep(-0.015, 0.015, d);
            // Two-tone cel shading lit from the top-left, plus a dark ink rim.
            let lit = if (x * 0.6 - y * 0.8) < -0.05 - d * 0.6 { 1.0 } else { 0.72 };
            let ink = 1.0 - smoothstep(0.035, 0.07, d);
            (lit * (1.0 - ink * 0.55), inside)
        }),
        Texture::SmokeSheet => sheet(4, 64, |x, y, k| {
            let r = (x * x + y * y).sqrt();
            let grow = 0.55 + 0.4 * k;
            let n = fbm(x * 2.0 + k * 0.7, y * 2.0 - k * 1.3, 7.0 + k, 4);
            let base = 1.0 - smoothstep(grow * 0.4, grow, r + n * 0.3);
            let erode = smoothstep(k * 0.75, k * 0.75 + 0.3, base + n * 0.25);
            let shade = 0.72 + 0.28 * (0.5 - 0.35 * (x - y)) + n * 0.2;
            (shade.clamp(0.0, 1.0), erode * (1.0 - k * 0.35))
        }),
        Texture::FlameSheet => sheet(4, 64, |x, y, k| {
            let h = (y + 1.0) * 0.5; // 0 at bottom, 1 at top
            let phase = k * 6.0;
            let n = fbm(x * 2.5, y * 2.0 - phase, 3.3, 3);
            let width = 0.62 * (1.0 - h).max(0.0).powf(0.65) * smoothstep(0.0, 0.18, h);
            let d = (x + n * 0.35 * h).abs() / width.max(1e-3);
            let body = 1.0 - smoothstep(0.55, 1.0, d + n * 0.25);
            let core = 1.0 - smoothstep(0.0, 0.6, d + h * 0.6);
            (0.7 + 0.3 * core, body * smoothstep(1.0, 0.75, h + n * 0.1))
        }),
        Texture::Streak => field(32, |x, y| {
            (1.0, (-(x / 0.12).powi(2)).exp() * (1.0 - y.abs()).max(0.0).sqrt())
        }),
        Texture::Leaf => field(64, |x, y| {
            let half = 0.48 * (1.0 - y * y).max(0.0) * (1.0 - 0.25 * y);
            let edge = smoothstep(0.0, 0.04, half - x.abs());
            let rib = (-(x / 0.035).powi(2)).exp() * 0.35;
            let shade = 0.82 + 0.18 * (x < 0.0) as u8 as f32 - rib;
            (shade, edge * (1.0 - smoothstep(0.9, 0.97, y.abs())))
        }),
        Texture::Confetti => field(32, |x, y| {
            let a = smoothstep(0.0, 0.08, 0.62 - x.abs()) * smoothstep(0.0, 0.08, 0.95 - y.abs());
            (0.85 + 0.15 * y, a)
        }),
        Texture::Droplet => field(64, |x, y| {
            // Round bottom, pointed top.
            let yy = (y + 0.35) / 1.3;
            let w = if yy < 0.0 {
                (0.5f32.powi(2) - (yy * 1.2).powi(2)).max(0.0).sqrt()
            } else {
                0.5 * (1.0 - yy).max(0.0).powf(1.4)
            };
            let a = smoothstep(0.0, 0.05, w - x.abs()) * (1.0 - smoothstep(0.95, 1.0, y));
            let hl = (-((x + 0.15).powi(2) + (y + 0.35).powi(2)) / 0.01).exp();
            ((0.75 + hl).min(1.0), a)
        }),
        Texture::Smear => field(96, |x, y| {
            let outer = 0.95 - (x * x + y * y).sqrt();
            let inner = 0.86 - ((x - 0.28).powi(2) + (y + 0.12).powi(2)).sqrt();
            let d = outer.min(-inner);
            // Taper the tips: fade towards the left-bottom end of the crescent.
            let ang = y.atan2(x);
            let taper = smoothstep(-2.6, -0.4, ang) * smoothstep(2.9, 1.6, ang);
            (1.0, smoothstep(-0.01, 0.02, d) * taper)
        }),
        Texture::Shard => field(64, |x, y| {
            // Triangle pointing up with two flat facets.
            let inside = y > -0.8 && y < 0.9 && x.abs() < (0.9 - y) * 0.55;
            let shade = if x < 0.0 { 1.0 } else { 0.68 };
            (shade, f32::from(u8::from(inside)))
        }),
        Texture::Hexagon => field(64, |x, y| {
            // Pointy-top hexagon split into three rhombi like an isometric cube.
            let r = 0.95;
            let inside = x.abs() <= 0.866 * r && y.abs() + x.abs() * 0.577 <= r;
            let shade = if y > 0.577 * x.abs() {
                1.0
            } else if x < 0.0 {
                0.8
            } else {
                0.62
            };
            (shade, f32::from(u8::from(inside)))
        }),
        Texture::PixelDot => pixels(&[
            "..oooo..", ".oXXXXo.", "oXXXXXXo", "oXXXXXXo", "oXXXXXXo", "oXXXXXXo", ".oXXXXo.",
            "..oooo..",
        ]),
        Texture::PixelSpark => pixels(&[
            "...X....", "...X....", "...o....", "XXoXoXX.", "...o....", "...X....", "...X....",
            "........",
        ]),
        Texture::PixelPuff => pixels(&[
            "................",
            "......####......",
            "....##XXXX##....",
            "...#XXXXXXXX#...",
            "..#XXXXXXXXXX#..",
            ".#XXXXXXXXXXXo#.",
            ".#XXXXXXXXXXoo#.",
            "#XXXXXXXXXXXooo#",
            "#XXXXXXXXXXoooo#",
            "#XXXXXXXXXooooo#",
            ".#XXXXXXooooooo#",
            ".#ooooooooooooo#",
            "..#ooooooooooo#.",
            "...##ooooooo##..",
            ".....#######....",
            "................",
        ]),
        Texture::Bubble => field(64, |x, y| {
            let r = (x * x + y * y).sqrt();
            let rim = (-((r - 0.85) / 0.08).powi(2)).exp();
            let fill = 0.12 * (1.0 - smoothstep(0.85, 0.9, r));
            let hl = (-((x + 0.35).powi(2) + (y - 0.4).powi(2)) / 0.012).exp();
            (1.0, (rim + fill + hl).min(1.0))
        }),
        Texture::Band => field_uv(16, 64, |_u, v| {
            let d = (v - 0.5) / 0.5;
            let a = (-(d / 0.45).powi(2)).exp();
            (1.0, a * (1.0 - smoothstep(0.85, 1.0, d.abs())))
        }),
        Texture::Electric => field_uv(128, 64, |u, v| {
            let wobble = fbm(u * 6.0, 0.5, 11.0, 3) * 0.28;
            let d = (v - 0.5 - wobble) / 0.5;
            let core = (-(d / 0.09).powi(2)).exp();
            let halo = (-(d / 0.4).powi(2)).exp() * 0.35;
            (1.0, (core + halo).min(1.0))
        }),
    }
}

/// Value noise helper over our gradient noise, roughly in `-1..=1`.
fn fbm(x: f32, y: f32, z: f32, octaves: u32) -> f32 {
    fbm3_grad(Vec3::new(x, y, z), 0x00C0_FFEE, octaves).0 * 1.6
}

/// Square image from `f(x, y) -> (luminance, alpha)` with `x, y` in `-1..=1`, `y` up.
fn field(size: u32, f: impl Fn(f32, f32) -> (f32, f32)) -> Image {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for j in 0..size {
        for i in 0..size {
            let x = (i as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let y = 1.0 - (j as f32 + 0.5) / size as f32 * 2.0;
            let (l, a) = f(x, y);
            push(&mut rgba, l, a);
        }
    }
    Image {
        width: size,
        height: size,
        rgba,
        nearest: false,
    }
}

/// Image from `f(u, v)` with `u, v` in `0..=1` (strip textures: `u` along, `v` across).
fn field_uv(w: u32, h: u32, f: impl Fn(f32, f32) -> (f32, f32)) -> Image {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for j in 0..h {
        for i in 0..w {
            let (l, a) = f((i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32);
            push(&mut rgba, l, a);
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
        nearest: false,
    }
}

/// A `grid x grid` flipbook; `f(x, y, k)` gets the frame's progress `k` in `0..=1`.
fn sheet(grid: u32, cell: u32, f: impl Fn(f32, f32, f32) -> (f32, f32)) -> Image {
    let size = grid * cell;
    let frames = (grid * grid - 1).max(1) as f32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for j in 0..size {
        for i in 0..size {
            let (ci, cj) = (i / cell, j / cell);
            let k = (cj * grid + ci) as f32 / frames;
            let x = ((i % cell) as f32 + 0.5) / cell as f32 * 2.0 - 1.0;
            let y = 1.0 - ((j % cell) as f32 + 0.5) / cell as f32 * 2.0;
            let (l, a) = f(x, y, k);
            // Fade cell borders so bilinear filtering never bleeds a neighbour frame in.
            let edge = smoothstep(1.0, 0.94, x.abs().max(y.abs()));
            push(&mut rgba, l, a * edge);
        }
    }
    Image {
        width: size,
        height: size,
        rgba,
        nearest: false,
    }
}

/// Pixel art from rows of `X` (bright), `o` (mid), `#` (dark outline) and `.` (clear).
fn pixels(rows: &[&str]) -> Image {
    let h = rows.len() as u32;
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u32;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in rows {
        let bytes = row.as_bytes();
        for i in 0..w as usize {
            let (l, a) = match bytes.get(i) {
                Some(b'X') => (1.0, 1.0),
                Some(b'o') => (0.7, 1.0),
                Some(b'#') => (0.38, 1.0),
                _ => (1.0, 0.0),
            };
            push(&mut rgba, l, a);
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
        nearest: true,
    }
}

fn push(out: &mut Vec<u8>, l: f32, a: f32) {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let l = q(l);
    out.extend_from_slice(&[l, l, l, q(a)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_texture_generates_a_visible_image() {
        for t in &Texture::BUILTIN {
            let img = generate(t);
            assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize, "{t:?}");
            let opaque = img.rgba.chunks(4).filter(|p| p[3] > 128).count();
            assert!(opaque > 4, "{t:?} is almost empty");
            assert_eq!(img.nearest, t.is_pixel_art(), "{t:?}");
        }
    }
}
