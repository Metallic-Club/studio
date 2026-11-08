//! The Metallic mark: the splash's emblem, the window's icon and the marks on the sign-in and projects screens. It is
//! `branding/metallic-mark.svg`: the Metallic logo (`branding/metallic-logo.svg`, #320) in white on a rounded square
//! in `#191a1e`, built into Studio and drawn at exactly the size it is shown at.

/// The mark's SVG.
const SVG: &str = include_str!("../../../branding/metallic-mark.svg");

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

/// The mark, `size` pixels square. Blank (and logged) in the unlikely case the built-in SVG cannot be drawn.
pub fn render(size: u32) -> Emblem {
    let size = size.max(1);
    let rgba = match draw(size) {
        Ok(rgba) => rgba,
        Err(error) => {
            log::warn!("emblem: the mark could not be drawn: {error}");
            vec![0; (size * size * 4) as usize]
        }
    };
    Emblem {
        width: size,
        height: size,
        rgba,
    }
}

/// The SVG drawn `size` pixels square, unmultiplied.
fn draw(size: u32) -> Result<Vec<u8>, String> {
    use resvg::{tiny_skia, usvg};

    let tree = usvg::Tree::from_str(SVG, &usvg::Options::default()).map_err(|error| error.to_string())?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("no image to draw in")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(emblem: &Emblem, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * emblem.width + x) * 4) as usize;
        emblem.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_mark_is_a_dark_square_with_the_white_logo() {
        let emblem = render(256);
        assert_eq!((emblem.width, emblem.height), (256, 256));
        assert_eq!(emblem.rgba.len(), 256 * 256 * 4);
        // Rounded corners are clear; the square is #191a1e around the logo.
        assert_eq!(pixel(&emblem, 0, 0)[3], 0);
        assert_eq!(pixel(&emblem, 255, 255)[3], 0);
        assert_eq!(pixel(&emblem, 128, 12), [0x19, 0x1A, 0x1E, 255], "above the logo");
        assert_eq!(pixel(&emblem, 12, 128), [0x19, 0x1A, 0x1E, 255], "left of the logo");
        // The logo's body, below and right of its middle, is white; the hole in its head is the square's colour.
        assert_eq!(pixel(&emblem, 150, 150), [255, 255, 255, 255], "the logo");
        let white = emblem
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[..3] == [255, 255, 255] && p[3] == 255)
            .count();
        let share = white as f32 / (256.0 * 256.0);
        assert!((0.15..0.4).contains(&share), "the logo covers {share} of the mark");
    }

    #[test]
    fn the_mark_holds_the_logo() {
        let logo = include_str!("../../../branding/metallic-logo.svg");
        let path = logo
            .split(" d=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        assert!(SVG.contains(path), "the mark draws branding/metallic-logo.svg's path");
        assert!(SVG.contains(r##"rx="64" fill="#191a1e""##));
    }

    #[test]
    fn every_size_draws() {
        for size in [1, 16, 32, 144, 512] {
            let emblem = render(size);
            assert_eq!(emblem.rgba.len(), (size * size * 4) as usize);
            assert!(emblem.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0), "{size}");
        }
    }
}
