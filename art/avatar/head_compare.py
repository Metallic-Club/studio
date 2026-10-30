"""Side-by-side of the head with 2 and 3 corner segments against the concept's head panel.

    blender -b --factory-startup --python art/avatar/head_compare.py
    python art/avatar/head_compare.py --compose

Renders with Workbench into out/rawB/, composes renders/phaseB/head_segments_v2.png.
"""
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "out", "rawB")
OUT = os.path.join(HERE, "renders", "phaseB")
VIEWS = {"front": 0, "side": 90, "34": 35}
CONCEPT_BOXES = {"front": (1305, 55, 1495, 255), "side": (1305, 275, 1495, 475), "34": (1305, 485, 1495, 685)}


def render():
    import bpy
    from mathutils import Vector
    sys.path.insert(0, HERE)
    import build_avatar as BA
    os.makedirs(RAW, exist_ok=True)
    for seg in (2, 3):
        BA.build(None, None, lod=0, head_seg=seg)
        sc = bpy.context.scene
        sc.render.engine = "BLENDER_WORKBENCH"
        sc.display.shading.light = "STUDIO"
        sc.display.shading.color_type = "SINGLE"
        sc.display.shading.single_color = (0.85, 0.78, 0.66)
        sc.display.shading.show_cavity = True
        sc.render.film_transparent = True
        sc.render.resolution_x = sc.render.resolution_y = 400
        cd = bpy.data.cameras.new("cam")
        cd.type = "ORTHO"
        cd.ortho_scale = 0.46
        cam = bpy.data.objects.new("cam", cd)
        sc.collection.objects.link(cam)
        sc.camera = cam
        t = Vector((0, 0, 1.36))
        for name, az in VIEWS.items():
            a = math.radians(az)
            d = Vector((math.sin(a), -math.cos(a), 0.05)).normalized()
            cam.location = t + d * 5
            cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
            sc.render.filepath = os.path.join(RAW, f"head_seg{seg}_{name}.png")
            bpy.ops.render.render(write_still=True)


def compose():
    from PIL import Image, ImageDraw, ImageFont
    concept = Image.open(os.path.join(HERE, "concept", "default-avatar-concept.png")).convert("RGBA")
    cell = 300
    sheet = Image.new("RGBA", (cell * 3 + 40, cell * 3 + 90), (232, 226, 218, 255))
    dr = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=18)
    for col, title in enumerate(("CONCEPT", "2 SEGMENTS", "3 SEGMENTS (chosen)")):
        dr.text((20 + col * cell + 10, 12), title, font=font, fill=(110, 80, 70))
    for row, name in enumerate(VIEWS):
        y = 44 + row * cell
        ims = [concept.crop(CONCEPT_BOXES[name])]
        ims += [Image.open(os.path.join(RAW, f"head_seg{s}_{name}.png")).convert("RGBA") for s in (2, 3)]
        for col, im in enumerate(ims):
            im.thumbnail((cell - 20, cell - 20))
            sheet.alpha_composite(im, (20 + col * cell + (cell - im.width) // 2, y + (cell - im.height) // 2))
    dr.text((20, cell * 3 + 56), "Head: front, side, 3/4. Workbench render, same scale for both models.",
            font=font, fill=(110, 80, 70))
    sheet.convert("RGB").save(os.path.join(OUT, "head_segments_v2.png"), optimize=True)
    print("wrote head_segments_v2.png")


if __name__ == "__main__":
    compose() if "--compose" in sys.argv else render()
