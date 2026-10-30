"""Owner-review stills: each concept view next to the same view of the model, in the concept's toon
look (Blender) and in the engine (real lighting, no outline).

    python art/avatar/owner_review.py

Needs Pillow, and:
- art/avatar/out/rawB/toon_*.png from `render_phaseB.py -- turnaround poses --passes toon`;
- art/avatar/out/engine/engine_*.png from
  `cargo run --release -p metallic_render --example avatar_review -- art/avatar/out/default_avatar.glb art/avatar/out/engine`.
Writes art/avatar/renders/v3/owner-review/:
  turnaround_toon.png, turnaround_engine.png    concept (top row) / model (bottom row), five views
  poses_toon.png, poses_engine.png              the three concept poses, concept / model
  engine_lods.png                               the five views at LOD0, LOD1 and LOD2 in the engine
With `--v2 DIR` (a rawB folder rendered by the v2 scripts) it also writes renders/v3/
before_after_toon.png and before_after_clay.png: concept / v2 / v3, five views.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
CONCEPT = os.path.join(HERE, "concept", "default-avatar-concept.png")
RAW = os.path.join(HERE, "out", "rawB")
ENGINE = os.path.join(HERE, "out", "engine")
VERSION = sys.argv[sys.argv.index("--version") + 1] if "--version" in sys.argv else "v4"
OUT = os.path.join(HERE, "renders", VERSION, "owner-review")

BG = (231, 227, 220)
INK = (110, 80, 70)
# Concept sheet crops (x0, y0, x1, y1) on the 1536 x 1024 sheet.
BODY = [("front", "FRONT", (22, 20, 322, 668)), ("front34", "FRONT 3/4", (318, 20, 592, 668)),
        ("side", "SIDE", (585, 20, 760, 668)), ("back", "BACK", (752, 20, 1030, 668)),
        ("back34", "BACK 3/4", (1020, 20, 1268, 668))]
POSES = [("pose_hand_on_hip", "HAND ON HIP", (930, 740, 1100, 1010)),
         ("pose_run", "RUN", (1100, 735, 1290, 1010)),
         ("pose_sit_back", "SITTING", (1240, 750, 1515, 1010))]


def figure_box(im, bg, tol=26):
    """Bounding box of the pixels that differ from the background colour."""
    px = im.convert("RGB").load()
    w, h = im.size
    xs, ys = [], []
    for y in range(0, h, 2):
        for x in range(0, w, 2):
            p = px[x, y]
            if sum(abs(a - b) for a, b in zip(p, bg)) > tol:
                xs.append(x)
                ys.append(y)
    if not xs:
        return (0, 0, w, h)
    return (max(0, min(xs) - 2), max(0, min(ys) - 2), min(w, max(xs) + 3), min(h, max(ys) + 3))


def concept_crop(box):
    im = Image.open(CONCEPT).convert("RGB").crop(box)
    return im.crop(figure_box(im, (231, 227, 220), 40))


def toon(name, raw=None, kind="toon"):
    path = os.path.join(raw or RAW, f"{kind}_{name}.png")
    if not os.path.exists(path):
        return None
    im = Image.open(path).convert("RGBA")
    im = im.crop(im.getbbox())
    base = Image.new("RGBA", im.size, BG + (255,))
    base.alpha_composite(im)
    return base.convert("RGB")


def engine(name, folder=None):
    path = os.path.join(folder or ENGINE, f"engine_{name}.png")
    if not os.path.exists(path):
        return None
    im = Image.open(path).convert("RGB")
    # The figure stands in the middle of a 480 x 800 frame; keep a margin of floor and sky.
    w, h = im.size
    return im.crop((int(w * 0.08), int(h * 0.06), int(w * 0.92), int(h * 0.94)))


def fit_h(im, h):
    return im.resize((max(1, round(im.width * h / im.height)), h), Image.LANCZOS)


def sheet(columns, title, path, cell_h=560, label_h=34, row_labels=("concept", "model")):
    """columns: (label, image per row...). Concept on top, then the model rows, same height."""
    font = ImageFont.load_default(size=18)
    n = len(columns[0]) - 1
    cells = [[fit_h(im, cell_h) if im is not None else None for im in col[1:]] for col in columns]
    widths = [max(im.width for im in col if im is not None) + 24 for col in cells]
    W = sum(widths) + 40
    H = 40 + (cell_h + label_h) * n + label_h
    s = Image.new("RGB", (W, H), BG)
    dr = ImageDraw.Draw(s)
    dr.text((20, 10), title, font=font, fill=INK)
    x = 20
    for col, ims, w in zip(columns, cells, widths):
        for r, im in enumerate(ims):
            if im is not None:
                s.paste(im, (x + (w - im.width) // 2, 40 + r * (cell_h + label_h)))
        tw = dr.textlength(col[0], font=font)
        dr.text((x + (w - tw) / 2, 40 + n * (cell_h + label_h) - label_h + 8), col[0], font=font, fill=INK)
        x += w
    for r in range(1, n):
        dr.text((20, 40 + r * (cell_h + label_h) - label_h + 6),
                f"{row_labels[r - 1]} (above)  /  {row_labels[r]} (below)", font=font, fill=INK)
    s.save(path, optimize=True)
    print("wrote", path)


def main():
    os.makedirs(OUT, exist_ok=True)
    toon_sheets = "--toon" in sys.argv   # Blender toon renders (out/rawB) of the same version
    if toon_sheets:
        sheet([(lab, concept_crop(box), toon(f"body_{n}")) for n, lab, box in BODY],
              f"Default avatar {VERSION}, toon look (Blender)", os.path.join(OUT, "turnaround_toon.png"))
    sheet([(lab, concept_crop(box), engine(f"{n}_lod0")) for n, lab, box in BODY],
          f"Default avatar {VERSION} in the engine (toon style, LOD0, rest pose with fists)",
          os.path.join(OUT, "turnaround_engine.png"))
    if toon_sheets:
        sheet([(lab, concept_crop(box), toon(n)) for n, lab, box in POSES],
              "Concept poses, toon look", os.path.join(OUT, "poses_toon.png"), cell_h=420)
    sheet([(lab, concept_crop(box), engine(n)) for n, lab, box in POSES],
          "Concept poses in the engine", os.path.join(OUT, "poses_engine.png"), cell_h=420)
    # LODs: three rows of the five engine views.
    if "--prev" in sys.argv:
        prev_dir, prev = sys.argv[sys.argv.index("--prev") + 1], sys.argv[sys.argv.index("--prev") + 2]
        sheet([(lab, concept_crop(box), engine(f"{n}_lod0", prev_dir), engine(f"{n}_lod0"))
               for n, lab, box in BODY],
              f"{prev} (as reviewed) vs {VERSION} in the engine", os.path.join(OUT, f"{prev}_vs_{VERSION}.png"),
              cell_h=460, row_labels=("concept", prev, VERSION))
    if "--v2" in sys.argv:
        v2 = sys.argv[sys.argv.index("--v2") + 1]
        for kind in ("toon", "clay"):
            sheet([(lab, concept_crop(box), toon(f"body_{n}", v2, kind), toon(f"body_{n}", None, kind))
                   for n, lab, box in BODY],
                  f"Before / after, {kind}: concept, v2, v3", os.path.join(HERE, "renders", "v3", f"before_after_{kind}.png"),
                  cell_h=460, row_labels=("concept", "v2", "v3"))
    rows = []
    for lod in (0, 1, 2):
        ims = [engine(f"{n}_lod{lod}") for n, _l, _b in BODY]
        if all(ims):
            rows.append([fit_h(im, 420) for im in ims])
    if rows:
        font = ImageFont.load_default(size=18)
        W = sum(im.width for im in rows[0]) + 120
        s = Image.new("RGB", (W, 420 * len(rows) + 20 * (len(rows) + 1)), BG)
        dr = ImageDraw.Draw(s)
        for r, row in enumerate(rows):
            y = 20 + r * 440
            dr.text((12, y + 200), f"LOD{r}", font=font, fill=INK)
            x = 100
            for im in row:
                s.paste(im, (x, y))
                x += im.width
        s.save(os.path.join(OUT, "engine_lods.png"), optimize=True)
        print("wrote engine_lods.png")


if __name__ == "__main__":
    main()
