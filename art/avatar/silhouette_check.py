"""Overlay the avatar's silhouette on the concept sheet's figure for the five body views.

    python art/avatar/silhouette_check.py

Needs Pillow and numpy, and the clay renders from render_turnaround.py in art/avatar/out/raw/.
For each view the concept figure is cut out of the sheet (background flood-filled from the crop
border), the render's alpha is scaled to the same height and bottom, and the horizontal offset with
the best overlap is used. Output: art/avatar/renders/phaseA/silhouette_<view>.png
  grey   = both
  orange = concept only (we are missing volume here)
  blue   = render only (we have extra volume here)
plus the intersection-over-union per view, and row-by-row width differences for the front view.
"""
import os
import sys

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
CONCEPT = os.path.join(HERE, "concept", "default-avatar-concept.png")
RAW = os.path.join(HERE, "out", "raw")
OUT = os.path.join(HERE, "renders", "phaseA")

# Crop boxes around each body figure on the 1536 x 1024 concept sheet (x0, y0, x1, y1).
CROPS = {
    "body_front": (22, 20, 322, 668),
    "body_front34": (318, 20, 592, 668),
    "body_side": (585, 20, 760, 668),
    "body_back": (752, 20, 1030, 668),
    "body_back34": (1020, 20, 1268, 668),
}


def grow(seed, allowed):
    r = seed & allowed
    while True:
        n = r.copy()
        n[1:] |= r[:-1]
        n[:-1] |= r[1:]
        n[:, 1:] |= r[:, :-1]
        n[:, :-1] |= r[:, 1:]
        n &= allowed
        if (n == r).all():
            return r
        r = n


def concept_mask(box):
    a = np.asarray(Image.open(CONCEPT).convert("RGB")).astype(int)
    x0, y0, x1, y1 = box
    a = a[y0:y1, x0:x1]
    R, G, B = a[..., 0], a[..., 1], a[..., 2]
    bg = ((R - B) < 22) & ((R + G + B) / 3 > 185)   # paper: low warmth, bright
    border = np.zeros_like(bg)
    border[0] = border[-1] = border[:, 0] = border[:, -1] = True
    fig = ~grow(border, bg)
    # keep the figure connected to the torso (drops neighbouring figures' edges)
    row = fig.shape[0] // 3
    xs = np.nonzero(fig[row])[0]
    seed = np.zeros_like(fig)
    seed[row, xs[np.argmin(abs(xs - fig.shape[1] // 2))]] = True
    return grow(seed, fig)


def trim(m):
    ys, xs = np.nonzero(m)
    return m[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def main():
    os.makedirs(OUT, exist_ok=True)
    report = []
    for view, box in CROPS.items():
        src = os.path.join(RAW, f"clay_{view}.png")
        if not os.path.exists(src):
            print("missing", src)
            continue
        cm = trim(concept_mask(box))
        H = cm.shape[0]
        alpha = np.asarray(Image.open(src).convert("RGBA"))[..., 3] > 127
        rm = trim(alpha)
        w = round(rm.shape[1] * H / rm.shape[0])
        rm = np.asarray(Image.fromarray((rm * 255).astype(np.uint8)).resize((w, H), Image.LANCZOS)) > 127
        W = max(cm.shape[1], w) + 80
        canvas_c = np.zeros((H, W), bool)
        cx0 = (W - cm.shape[1]) // 2
        canvas_c[:, cx0:cx0 + cm.shape[1]] = cm
        best = None
        for dx in range(-40, 41):
            canvas_r = np.zeros((H, W), bool)
            rx0 = (W - w) // 2 + dx
            canvas_r[:, rx0:rx0 + w] = rm
            iou = (canvas_c & canvas_r).sum() / (canvas_c | canvas_r).sum()
            if best is None or iou > best[0]:
                best = (iou, dx, canvas_r)
        iou, dx, canvas_r = best
        img = np.full((H, W, 3), 245, np.uint8)
        img[canvas_c & canvas_r] = (175, 175, 175)
        img[canvas_c & ~canvas_r] = (240, 140, 40)
        img[~canvas_c & canvas_r] = (60, 120, 230)
        Image.fromarray(img).save(os.path.join(OUT, f"silhouette_{view}.png"), optimize=True)
        report.append(f"{view}: IoU {iou:.3f} (offset {dx:+d} px, concept height {H} px)")
        if "-v" in sys.argv:
            for frac in np.arange(0.0, 1.0001, 0.025):
                y = min(H - 1, int(frac * H))
                c, r = np.nonzero(canvas_c[y])[0], np.nonzero(canvas_r[y])[0]
                cs = f"{c.min()}-{c.max()}" if len(c) else "-"
                rs = f"{r.min()}-{r.max()}" if len(r) else "-"
                print(f"  {view} {frac:.3f} concept {cs:>9} ({len(c):3d})  render {rs:>9} ({len(r):3d})")
    print("\n".join(report))
    with open(os.path.join(OUT, "silhouette_iou.txt"), "w") as f:
        f.write("\n".join(report) + "\n")


if __name__ == "__main__":
    main()
