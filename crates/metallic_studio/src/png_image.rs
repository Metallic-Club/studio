//! Minimal PNG decoding to RGBA8, used for the window icon and template previews.

use std::io::Cursor;

use crate::StudioError;

/// A decoded image: tightly packed RGBA8 rows.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// The size of a PNG from its header, without decoding any pixels.
pub fn png_dimensions(bytes: &[u8], label: &str) -> Result<(u32, u32), StudioError> {
    let reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|e| StudioError::Image {
            path: label.to_owned(),
            message: e.to_string(),
        })?;
    let info = reader.info();
    Ok((info.width, info.height))
}

/// Decodes PNG `bytes`. `label` names the source in error messages.
pub fn decode_png(bytes: &[u8], label: &str) -> Result<RgbaImage, StudioError> {
    let fail = |message: String| StudioError::Image {
        path: label.to_owned(),
        message,
    };
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| fail(e.to_string()))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| fail("image too large".into()))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| fail(e.to_string()))?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err(fail("indexed colour was not expanded".into())),
    };
    Ok(RgbaImage {
        width: info.width,
        height: info.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_brand_icon() {
        let bytes = include_bytes!("../../../branding/placeholder-icon.png");
        let img = decode_png(bytes, "icon").expect("decode");
        assert_eq!((img.width, img.height), (256, 256));
        assert_eq!(img.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn dimensions_come_from_the_header() {
        let bytes = include_bytes!("../../../branding/placeholder-icon.png");
        assert_eq!(png_dimensions(bytes, "icon").expect("header"), (256, 256));
        assert!(png_dimensions(b"nope", "bad").is_err());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_png(b"nope", "bad").is_err());
    }
}
