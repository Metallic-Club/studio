//! The colours written in a script (#545): `Color3.new(r, g, b)` (0 to 1), `Color3.fromRGB(r, g, b)` (0 to 255),
//! `Color3.fromHSV(h, s, v)` (0 to 1) and `Color3.fromHex("#rrggbb")`, with literal arguments. The code field shows a
//! swatch of each before it, which opens Studio's colour picker; a colour picked is written back in the same form.

use super::lexer::{Token, TokenKind, lex};
use crate::panels::color_picker::{Hsv, hex, parse_hex};

/// How a colour is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `Color3.new(r, g, b)`, each 0 to 1.
    Unit,
    /// `Color3.fromRGB(r, g, b)`, each 0 to 255.
    Rgb,
    /// `Color3.fromHSV(h, s, v)`, each 0 to 1.
    Hsv,
    /// `Color3.fromHex("#rrggbb")`.
    Hex,
}

/// A colour written in a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    /// How it is written.
    pub form: Form,
    /// The byte where `Color3` starts: the swatch shows before it.
    pub start: usize,
    /// The bytes of its arguments, between the brackets: what a colour picked replaces.
    pub args: std::ops::Range<usize>,
    /// The colour, in 0-255 sRGB channels.
    pub rgb: [u8; 3],
    /// Its hue (0 to 1), as written for [`Form::Hsv`] (a grey keeps it), else the colour's own.
    pub hue: f32,
}

/// The colours written in `source`, in order.
pub fn find(source: &str) -> Vec<Swatch> {
    let tokens: Vec<Token> = lex(source)
        .into_iter()
        .filter(|token| token.kind != TokenKind::Comment)
        .collect();
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != TokenKind::Name || token.text(source) != "Color3" {
            continue;
        }
        let member = index
            .checked_sub(1)
            .is_some_and(|before| tokens[before].is(source, ".") || tokens[before].is(source, ":"));
        if member {
            continue;
        }
        if let Some(swatch) = call(source, &tokens[index..]) {
            found.push(swatch);
        }
    }
    found
}

/// The colour of the call `tokens` starts (`Color3 . name ( args )`), if it is one with literal arguments.
fn call(source: &str, tokens: &[Token]) -> Option<Swatch> {
    let [color3, dot, name, open, ..] = tokens else {
        return None;
    };
    if !dot.is(source, ".") || !open.is(source, "(") {
        return None;
    }
    let form = match name.text(source) {
        "new" => Form::Unit,
        "fromRGB" => Form::Rgb,
        "fromHSV" => Form::Hsv,
        "fromHex" => Form::Hex,
        _ => return None,
    };
    let close = tokens.iter().position(|token| token.is(source, ")"))?;
    let args = &tokens[4..close];
    let inside = open.end..tokens[close].start;
    let (rgb, hue) = if form == Form::Hex {
        let [string] = args else {
            return None;
        };
        let text = string.text(source);
        let quote = text.chars().next()?;
        if string.kind != TokenKind::String || !matches!(quote, '"' | '\'') || string.unterminated {
            return None;
        }
        let rgb = parse_hex(text.get(1..text.len() - 1)?)?;
        (rgb, Hsv::of(rgb, 0.0).h)
    } else {
        let [a, comma, b, comma2, c] = args else {
            return None;
        };
        if !comma.is(source, ",") || !comma2.is(source, ",") {
            return None;
        }
        let number = |token: &Token| {
            (token.kind == TokenKind::Number)
                .then(|| token.text(source).parse::<f32>().ok())
                .flatten()
        };
        let values = [number(a)?, number(b)?, number(c)?];
        let to_byte = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;
        match form {
            Form::Unit => {
                let rgb = values.map(to_byte);
                (rgb, Hsv::of(rgb, 0.0).h)
            }
            Form::Rgb => {
                let rgb = values.map(|channel| channel.clamp(0.0, 255.0).round() as u8);
                (rgb, Hsv::of(rgb, 0.0).h)
            }
            Form::Hsv | Form::Hex => {
                let [h, s, v] = values.map(|value| value.clamp(0.0, 1.0));
                (Hsv { h, s, v }.rgb8(), h)
            }
        }
    };
    Some(Swatch {
        form,
        start: color3.start,
        args: inside,
        rgb,
        hue,
    })
}

/// The arguments that write `rgb` in `swatch`'s form, a hex string in the quote it had (`old` being its arguments as
/// written).
pub fn write(swatch: &Swatch, old: &str, rgb: [u8; 3]) -> String {
    let unit = |value: f32| {
        let text = format!("{:.3}", value.clamp(0.0, 1.0));
        let text = text.trim_end_matches('0').trim_end_matches('.');
        if text.is_empty() {
            "0".to_owned()
        } else {
            text.to_owned()
        }
    };
    match swatch.form {
        Form::Unit => rgb.map(|channel| unit(f32::from(channel) / 255.0)).join(", "),
        Form::Rgb => rgb.map(|channel| channel.to_string()).join(", "),
        Form::Hsv => {
            let hsv = Hsv::of(rgb, swatch.hue);
            [hsv.h, hsv.s, hsv.v].map(unit).join(", ")
        }
        Form::Hex => {
            let trimmed = old.trim();
            let quote = trimmed
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\''))
                .unwrap_or('"');
            let hash = trimmed.get(1..).is_some_and(|inner| inner.starts_with('#'));
            let digits = hex(rgb).to_uppercase();
            let digits = if hash {
                digits
            } else {
                digits.trim_start_matches('#').to_owned()
            };
            format!("{quote}{digits}{quote}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(source: &str) -> Swatch {
        let found = find(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn each_form_is_found_with_its_colour() {
        let source = "local a = Color3.new(1, 0.5, 0)";
        let swatch = one(source);
        assert_eq!((swatch.form, swatch.rgb), (Form::Unit, [255, 128, 0]));
        assert_eq!(swatch.start, source.find("Color3").unwrap());
        assert_eq!(&source[swatch.args.clone()], "1, 0.5, 0");
        assert_eq!(one("x = Color3.fromRGB(12, 34, 255)").rgb, [12, 34, 255]);
        let hsv = one("x = Color3.fromHSV(0.5, 1, 1)");
        assert_eq!((hsv.form, hsv.rgb, hsv.hue), (Form::Hsv, [0, 255, 255], 0.5));
        assert_eq!(one("x = Color3.fromHex(\"#3BC9A0\")").rgb, [0x3B, 0xC9, 0xA0]);
        assert_eq!(one("x = Color3.fromHex('fff')").rgb, [255, 255, 255]);
        // Several, in order.
        let two = find("a = Color3.new(0, 0, 0) b = Color3.fromRGB(255, 255, 255)");
        assert_eq!(
            two.iter().map(|swatch| swatch.form).collect::<Vec<_>>(),
            [Form::Unit, Form::Rgb]
        );
    }

    #[test]
    fn only_literal_colours_are_found() {
        for source in [
            "Color3.new(r, g, b)",
            "Color3.new()",
            "Color3.fromRGB(1, 2)",
            "Color3.fromHex(name)",
            "Color3.fromHex(\"nope\")",
            "Color3.lerp(1, 2, 3)",
            "x.Color3.new(1, 1, 1)",
            "-- Color3.new(1, 1, 1)",
            "Color3.new(1, 1, 1 + 0)",
        ] {
            assert!(find(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn a_colour_picked_is_written_in_its_form() {
        let write_to = |source: &str, rgb: [u8; 3]| {
            let swatch = one(source);
            let mut out = source.to_owned();
            let with = write(&swatch, &source[swatch.args.clone()], rgb);
            out.replace_range(swatch.args.clone(), &with);
            out
        };
        assert_eq!(
            write_to("Color3.new(1, 1, 1)", [255, 128, 0]),
            "Color3.new(1, 0.502, 0)"
        );
        assert_eq!(
            write_to("Color3.fromRGB(0, 0, 0)", [12, 34, 255]),
            "Color3.fromRGB(12, 34, 255)"
        );
        assert_eq!(
            write_to("Color3.fromHSV(0, 0, 0)", [0, 255, 255]),
            "Color3.fromHSV(0.5, 1, 1)"
        );
        // A grey keeps the hue it was written with.
        assert_eq!(
            write_to("Color3.fromHSV(0.25, 1, 1)", [128, 128, 128]),
            "Color3.fromHSV(0.25, 0, 0.502)"
        );
        assert_eq!(
            write_to("Color3.fromHex(\"#000000\")", [0x3B, 0xC9, 0xA0]),
            "Color3.fromHex(\"#3BC9A0\")"
        );
        assert_eq!(
            write_to("Color3.fromHex('000')", [255, 0, 0]),
            "Color3.fromHex('FF0000')"
        );
    }
}
