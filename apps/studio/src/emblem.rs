//! The Metallic mark: the splash's emblem and the window's icon. It is the website's mark
//! (`branding/metallic-mark.svg`, the website's `public/favicon.svg`): a radius-10 square in `#191a1e` holding a white
//! geometric "M", on a 40 × 40 grid.
//!
//! The shape is a few straight edges and four arcs, so it is drawn here from the same numbers as the SVG rather than
//! through an SVG library: four samples per pixel on each axis.

/// The mark's grid: the SVG's `viewBox` is `0 0 GRID GRID`.
const GRID: f32 = 40.0;
/// The square's corner radius on the grid.
const RADIUS: f32 = 10.0;
/// The square's colour, `#191a1e`.
const SQUARE: [u8; 3] = [0x19, 0x1A, 0x1E];
/// The "M", on the grid: the SVG's `M9 29V11h5l6 10 6-10h5v18h-5V20l-6 9-6-9v9z`.
const LETTER: [(f32, f32); 12] = [
    (9.0, 29.0),
    (9.0, 11.0),
    (14.0, 11.0),
    (20.0, 21.0),
    (26.0, 11.0),
    (31.0, 11.0),
    (31.0, 29.0),
    (26.0, 29.0),
    (26.0, 20.0),
    (20.0, 29.0),
    (14.0, 20.0),
    (14.0, 29.0),
];
/// Samples per pixel on each axis.
const SAMPLES: u32 = 4;

/// The mark as 8-bit RGBA pixels, unmultiplied.
#[derive(Clone, Debug)]
pub struct Emblem {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixels, row by row from the top.
    pub rgba: Vec<u8>,
}

/// The mark, `size` pixels square.
pub fn render(size: u32) -> Emblem {
    let size = size.max(1);
    let scale = GRID / size as f32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut square, mut letter) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let py = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    if in_square(px, py) {
                        square += 1;
                        if in_letter(px, py) {
                            letter += 1;
                        }
                    }
                }
            }
            let total = SAMPLES * SAMPLES;
            if square == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // The letter's share of the covered samples, blended from the square's colour to white.
            let mix = |c: u8| (u32::from(c) * (square - letter) + 255 * letter) / square;
            rgba.extend_from_slice(&[
                mix(SQUARE[0]) as u8,
                mix(SQUARE[1]) as u8,
                mix(SQUARE[2]) as u8,
                (255 * square / total) as u8,
            ]);
        }
    }
    Emblem {
        width: size,
        height: size,
        rgba,
    }
}

/// Inside the rounded square.
fn in_square(x: f32, y: f32) -> bool {
    let cx = x.clamp(RADIUS, GRID - RADIUS);
    let cy = y.clamp(RADIUS, GRID - RADIUS);
    (0.0..=GRID).contains(&x) && (0.0..=GRID).contains(&y) && (x - cx).powi(2) + (y - cy).powi(2) <= RADIUS * RADIUS
}

/// Inside the "M" (even-odd rule).
fn in_letter(x: f32, y: f32) -> bool {
    let mut inside = false;
    for (&(ax, ay), &(bx, by)) in LETTER.iter().zip(LETTER.iter().cycle().skip(1)) {
        if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(emblem: &Emblem, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * emblem.width + x) * 4) as usize;
        emblem.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_mark_is_a_dark_square_with_a_white_m() {
        let emblem = render(80);
        assert_eq!((emblem.width, emblem.height), (80, 80));
        assert_eq!(emblem.rgba.len(), 80 * 80 * 4);
        // Rounded corners are clear; the square is #191a1e; the M's legs are white.
        assert_eq!(pixel(&emblem, 0, 0)[3], 0);
        assert_eq!(pixel(&emblem, 79, 79)[3], 0);
        assert_eq!(pixel(&emblem, 40, 4), [0x19, 0x1A, 0x1E, 255]);
        assert_eq!(pixel(&emblem, 22, 40), [255, 255, 255, 255], "left leg");
        assert_eq!(pixel(&emblem, 57, 40), [255, 255, 255, 255], "right leg");
        assert_eq!(pixel(&emblem, 40, 30), [0x19, 0x1A, 0x1E, 255], "above the V");
        assert_eq!(pixel(&emblem, 40, 50), [255, 255, 255, 255], "the V");
    }

    #[test]
    fn the_numbers_are_the_svgs() {
        let svg = include_str!("../../../branding/metallic-mark.svg");
        assert!(svg.contains(r#"viewBox="0 0 40 40""#));
        assert!(svg.contains(r##"rx="10" fill="#191a1e""##));
        assert!(svg.contains(r##"d="M9 29V11h5l6 10 6-10h5v18h-5V20l-6 9-6-9v9z" fill="#fff""##));
    }
}
