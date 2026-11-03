//! The overlay's built-in 5 × 7 bitmap font: printable ASCII, lower case drawn as upper case. Small enough to live
//! in each glyph quad's instance data, so the overlay needs no texture and no text-rendering crate.

/// Columns of a glyph cell.
pub const GLYPH_COLUMNS: u32 = 5;
/// Rows of a glyph cell.
pub const GLYPH_ROWS: u32 = 7;

/// A glyph's rows, top first; bit 4 of each row is the leftmost pixel.
type Rows = [u8; 7];

/// The rows of `c`, or `None` for a character the font does not have.
const fn rows(c: u8) -> Option<Rows> {
    let c = c.to_ascii_uppercase();
    Some(match c {
        b' ' => [0; 7],
        b'0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        b'1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        b'3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        b'4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        b'7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        b'8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        b'9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        b'A' => [0b01110, 0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001],
        b'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        b'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        b'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        b'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        b'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        b'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        b'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        b'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        b'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        b'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        b'N' => [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001],
        b'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        b'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        b'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        b'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        b'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        b'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        b'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        b'Y' => [0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100],
        b'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        b'.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        b',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        b':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        b';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b00100, 0b01000],
        b'/' => [0, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0],
        b'\\' => [0, 0b10000, 0b01000, 0b00100, 0b00010, 0b00001, 0],
        b'%' => [0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011],
        b'-' => [0, 0, 0, 0b11111, 0, 0, 0],
        b'+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        b'*' => [0, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0],
        b'=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 0b11111],
        b'(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        b')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        b'[' => [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110],
        b']' => [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110],
        b'<' => [0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010],
        b'>' => [0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000],
        b'?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        b'!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        b'\'' => [0b01100, 0b00100, 0b01000, 0, 0, 0, 0],
        b'"' => [0b01010, 0b01010, 0b01010, 0, 0, 0, 0],
        b'#' => [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010],
        b'|' => [0b00100; 7],
        b'@' => [0b01110, 0b10001, 0b00001, 0b01101, 0b10101, 0b10101, 0b01110],
        b'&' => [0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101],
        b'$' => [0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100],
        b'^' => [0b00100, 0b01010, 0b10001, 0, 0, 0, 0],
        b'~' => [0, 0, 0b01000, 0b10101, 0b00010, 0, 0],
        b'`' => [0b01000, 0b00100, 0b00010, 0, 0, 0, 0],
        b'{' => [0b00010, 0b00100, 0b00100, 0b01000, 0b00100, 0b00100, 0b00010],
        b'}' => [0b01000, 0b00100, 0b00100, 0b00010, 0b00100, 0b00100, 0b01000],
        _ => return None,
    })
}

/// Packs a glyph's 35 pixels as the shader reads them: pixel `row * 5 + column` (column 0 on the left) is bit
/// `n` of the first word for `n < 20` (rows 0 to 3) and bit `n - 20` of the second (rows 4 to 6).
const fn pack(rows: Rows) -> [u32; 2] {
    let mut words = [0u32; 2];
    let mut row = 0;
    while row < GLYPH_ROWS as usize {
        let mut column = 0;
        while column < GLYPH_COLUMNS as usize {
            if rows[row] & (1 << (4 - column)) != 0 {
                let bit = row * 5 + column;
                if bit < 20 {
                    words[0] |= 1 << bit;
                } else {
                    words[1] |= 1 << (bit - 20);
                }
            }
            column += 1;
        }
        row += 1;
    }
    words
}

/// Every printable ASCII glyph, packed, from `' '` (0x20) to `'~'` (0x7E).
const TABLE: [[u32; 2]; 95] = {
    let mut table = [[0u32; 2]; 95];
    let missing = match rows(b'?') {
        Some(rows) => pack(rows),
        None => [0; 2],
    };
    let mut i = 0;
    while i < table.len() {
        table[i] = match rows(i as u8 + b' ') {
            Some(rows) => pack(rows),
            None => missing,
        };
        i += 1;
    }
    table
};

/// The packed pixels of `c`; `?` for a character the font does not have, nothing (all zeros) for a space.
pub(crate) fn glyph(c: char) -> [u32; 2] {
    let code = c as u32;
    if (0x20..0x7F).contains(&code) {
        TABLE[(code - 0x20) as usize]
    } else {
        TABLE[(b'?' - b' ') as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixel at `row`, `column` of a packed glyph.
    fn pixel(words: [u32; 2], row: u32, column: u32) -> bool {
        let bit = row * GLYPH_COLUMNS + column;
        if bit < 20 {
            words[0] & (1 << bit) != 0
        } else {
            words[1] & (1 << (bit - 20)) != 0
        }
    }

    #[test]
    fn every_printable_character_has_a_glyph_and_lower_case_is_upper_case() {
        for c in b'!'..=b'~' {
            assert!(rows(c).is_some(), "{:?} has no glyph", c as char);
        }
        assert_eq!(glyph('m'), glyph('M'));
        assert_eq!(glyph(' '), [0, 0]);
        assert_eq!(glyph('\u{e9}'), glyph('?'), "outside ASCII draws a question mark");
    }

    #[test]
    fn packing_keeps_every_pixel_in_place() {
        // '1': the top row has only its middle pixel, the bottom row three.
        let one = glyph('1');
        assert!(pixel(one, 0, 2));
        assert!(!pixel(one, 0, 0) && !pixel(one, 0, 4));
        assert!(pixel(one, 6, 1) && pixel(one, 6, 2) && pixel(one, 6, 3));
        assert!(!pixel(one, 6, 0) && !pixel(one, 6, 4));
        // No glyph uses bits past the 35 pixels.
        for c in ' '..='~' {
            assert_eq!(glyph(c)[1] >> 15, 0, "{c:?}");
            assert_eq!(glyph(c)[0] >> 20, 0, "{c:?}");
        }
    }
}
