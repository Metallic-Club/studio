#!/usr/bin/env python3
"""Writes the WOFF2 copies of Studio's bundled text fonts for the web surfaces (the Launcher pages).

The source is crates/metallic_ui_theme/assets/fonts/*.ttf (already subset, see NOTICE.txt there).
WOFF2 is a lossless, Brotli-compressed wrapper of the same font, so every surface draws the same
glyphs. Needs fontTools and brotli: `python -m pip install fonttools brotli`.

    python tools/gen_web_fonts.py          # write the .woff2 files
    python tools/gen_web_fonts.py --check  # fail if a committed .woff2 is missing or out of date
"""

import argparse
import io
import sys
from pathlib import Path

from fontTools.ttLib import TTFont

FONTS = Path(__file__).resolve().parent.parent / "crates" / "metallic_ui_theme" / "assets" / "fonts"
# Only the fonts a web page draws text with. Noto Sans (Greek and Cyrillic fallback) and Phosphor
# are left to Studio: the pages fall back to the system font for those scripts and use no icon font.
NAMES = ["Archivo-Variable", "JetBrainsMono-Variable"]


def woff2_bytes(ttf: Path) -> bytes:
    # Keep the head table's dates, so the output depends on the input only.
    font = TTFont(ttf, recalcTimestamp=False)
    font.flavor = "woff2"
    out = io.BytesIO()
    font.save(out, reorderTables=True)
    return out.getvalue()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="compare instead of writing")
    args = parser.parse_args()
    stale = []
    for name in NAMES:
        data = woff2_bytes(FONTS / f"{name}.ttf")
        target = FONTS / f"{name}.woff2"
        if args.check:
            if not target.exists() or target.read_bytes() != data:
                stale.append(target.name)
        else:
            target.write_bytes(data)
            print(f"{target.name}: {len(data)} bytes")
    if stale:
        print("out of date: " + ", ".join(stale) + "; run python tools/gen_web_fonts.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
