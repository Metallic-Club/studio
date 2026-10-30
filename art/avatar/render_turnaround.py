"""Turnaround stills of the default avatar (Phase A), in grey clay and in the concept's toon look.

Two steps, one file:
  1. Render (inside Blender, EEVEE, low samples):
       blender -b --factory-startup --python art/avatar/render_turnaround.py -- [--only VIEW ...] [--passes clay toon]
     Builds the mesh with build_avatar.py, then writes transparent RGBA renders to art/avatar/out/raw/.
  2. Compose (plain Python 3 with Pillow and numpy):
       python art/avatar/render_turnaround.py --compose
     Puts each raw render on its background into art/avatar/renders/phaseA/ and lays out the
     contact sheets like the concept sheet.

The camera orbits the figure; lights ride with the camera so every view is lit from the same screen
direction, the way the concept sheet is drawn. Azimuth 0 = in front of the figure, positive azimuth
moves the camera toward the figure's right side.
"""
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "out", "raw")
OUT = os.path.join(HERE, "renders", "phaseA")

# name: (part groups or None for the whole body, target, ortho size, azimuth, elevation, (w, h)).
# A target is a point, or (vertex group, offset) = the centre of that group's bounding box plus offset.
BODY_RES = (520, 1000)
VIEWS = {
    "body_front": (None, (0, 0, 0.80), 1.70, 0, 8, BODY_RES),
    "body_front34": (None, (0, 0, 0.80), 1.70, 30, 8, BODY_RES),   # the sheet's 3/4 is turned about 30 degrees
    "body_side": (None, (0, 0, 0.80), 1.70, 90, 8, BODY_RES),
    "body_back": (None, (0, 0, 0.80), 1.70, 180, 8, BODY_RES),
    "body_back34": (None, (0, 0, 0.80), 1.70, 225, 8, BODY_RES),
    "head_front": (("Head", "Neck"), (0, 0.0, 1.33), 0.50, 0, 4, (480, 480)),
    "head_side": (("Head", "Neck"), (0, 0.0, 1.33), 0.50, -90, 4, (480, 480)),
    "head_34": (("Head", "Neck"), (0, 0.0, 1.33), 0.50, -45, 4, (480, 480)),
    "hand_back": (("Hand.L", "Arm.L"), ("Hand.L", (0, 0, 0.012)), 0.25, -90, 8, (420, 420)),
    "hand_front": (("Hand.L", "Arm.L"), ("Hand.L", (0, 0, 0.012)), 0.25, 0, 8, (420, 420)),
    "hand_palm": (("Hand.L", "Arm.L"), ("Hand.L", (0, 0, 0.012)), 0.25, 90, 8, (420, 420)),
    "foot_front": (("Boot.R", "Leg.R"), ("Boot.R", (0, 0, 0.075)), 0.40, 0, 20, (420, 420)),
    "foot_side": (("Boot.R", "Leg.R"), ("Boot.R", (0, 0, 0.075)), 0.40, 90, 10, (420, 420)),
    "foot_sole": (("Boot.R",), ("Boot.R", (0, 0, 0)), 0.40, 0, -90, (420, 420)),
}
OUTLINE_PX = 3.4                   # outline width in output pixels
INNER_LINE_PX = 1.6                # interior construction lines (Freestyle creases)

BG_CLAY = (222, 222, 222)
BG_TOON = (231, 227, 220)          # the concept's warm off-white
TOON_BANDS = [(200, 180, 158), (219, 203, 179), (234, 221, 198)]  # sampled from the concept
TOON_LINE = (92, 52, 42)           # dark brown outline


# ======================================================================== Blender side

def srgb_to_lin(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lin(rgb):
    return tuple(srgb_to_lin(c) for c in rgb) + (1.0,)


def blender_main(argv):
    import argparse
    import bpy
    from mathutils import Matrix, Vector

    sys.path.insert(0, HERE)
    import build_avatar

    ap = argparse.ArgumentParser()
    ap.add_argument("--only", nargs="*", default=None)
    ap.add_argument("--passes", nargs="*", default=["clay", "toon"])
    ap.add_argument("--samples", type=int, default=16)
    a = ap.parse_args(argv)

    avatar, tris = build_avatar.build(None, None)
    scene = bpy.context.scene
    for eng in ("BLENDER_EEVEE", "BLENDER_EEVEE_NEXT"):
        try:
            scene.render.engine = eng
            break
        except TypeError:
            continue
    try:
        scene.eevee.taa_render_samples = a.samples
    except AttributeError:
        pass
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"

    world = bpy.data.worlds.new("World")
    scene.world = world
    world.use_nodes = True
    bg = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")

    cam_data = bpy.data.cameras.new("Cam")
    cam_data.type = "ORTHO"
    cam_data.clip_start, cam_data.clip_end = 0.01, 50
    cam = bpy.data.objects.new("Cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam

    def sun(name, strength, rot_deg, angle=2.0):
        d = bpy.data.lights.new(name, "SUN")
        d.energy = strength
        d.angle = math.radians(angle)
        o = bpy.data.objects.new(name, d)
        scene.collection.objects.link(o)
        o.parent = cam
        o.rotation_euler = [math.radians(r) for r in rot_deg]
        return o

    # Camera-relative suns (camera looks down its -Z). Key from upper right, fill from left, rim behind.
    key = sun("Key", 1.0, (-38, 32, 0))
    fill = sun("Fill", 0.0, (-10, -55, 0), 8)
    rim = sun("Rim", 0.0, (-150, -30, 0), 4)

    # Part objects for close-ups, cut from the joined mesh by vertex group.
    def part_object(groups):
        ob = avatar.copy()
        ob.data = avatar.data.copy()
        ob.name = "Part_" + "_".join(groups)
        scene.collection.objects.link(ob)
        idx = {ob.vertex_groups[g].index for g in groups}
        import bmesh
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        dl = bm.verts.layers.deform.active
        drop = [v for v in bm.verts if not any(k in v[dl] for k in idx)]
        bmesh.ops.delete(bm, geom=drop, context="VERTS")
        bm.to_mesh(ob.data)
        bm.free()
        ob.hide_render = True
        return ob

    parts = {}
    for name, (groups, *_rest) in VIEWS.items():
        if groups and groups not in parts:
            parts[groups] = part_object(groups)
    objects = [avatar] + list(parts.values())

    clay_mats = [m for m in avatar.data.materials]

    def toon_material():
        m = bpy.data.materials.new("M_Toon")
        nt = m.node_tree
        for n in list(nt.nodes):
            nt.nodes.remove(n)
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        dif = nt.nodes.new("ShaderNodeBsdfDiffuse")
        dif.inputs["Color"].default_value = (1, 1, 1, 1)
        s2r = nt.nodes.new("ShaderNodeShaderToRGB")
        ramp = nt.nodes.new("ShaderNodeValToRGB")
        cr = ramp.color_ramp
        cr.interpolation = "CONSTANT"
        cr.elements[0].position = 0.0
        cr.elements[0].color = lin(TOON_BANDS[0])
        cr.elements[1].position = 0.16
        cr.elements[1].color = lin(TOON_BANDS[1])
        e = cr.elements.new(0.42)
        e.color = lin(TOON_BANDS[2])
        em = nt.nodes.new("ShaderNodeEmission")
        nt.links.new(dif.outputs[0], s2r.inputs[0])
        nt.links.new(s2r.outputs["Color"], ramp.inputs["Fac"])
        nt.links.new(ramp.outputs["Color"], em.inputs["Color"])
        nt.links.new(em.outputs[0], out.inputs["Surface"])
        return m

    def line_material():
        m = bpy.data.materials.new("M_Outline")
        nt = m.node_tree
        for n in list(nt.nodes):
            nt.nodes.remove(n)
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        em = nt.nodes.new("ShaderNodeEmission")
        em.inputs["Color"].default_value = lin(TOON_LINE)
        nt.links.new(em.outputs[0], out.inputs["Surface"])
        m.use_backface_culling = True
        return m

    toon, line = toon_material(), line_material()

    def set_pass(p):
        for ob in objects:
            for m in [x for x in ob.modifiers if x.type == "SOLIDIFY"]:
                ob.modifiers.remove(m)
            ob.data.materials.clear()
            if p == "clay":
                for m in clay_mats:
                    ob.data.materials.append(m)
            else:
                ob.data.materials.append(toon)
                ob.data.materials.append(line)
                for poly in ob.data.polygons:
                    poly.material_index = 0
        if p == "clay":
            bg.inputs["Color"].default_value = (0.22, 0.22, 0.22, 1)
            bg.inputs["Strength"].default_value = 1.0
            key.data.energy, fill.data.energy, rim.data.energy = 4.0, 0.45, 1.4
            key.data.use_shadow = True
            scene.render.use_freestyle = False
        else:
            bg.inputs["Color"].default_value = (0, 0, 0, 1)
            bg.inputs["Strength"].default_value = 0.0
            # pi makes the diffuse value equal to the cosine to the light, so the ramp stops are angles.
            key.data.energy, fill.data.energy, rim.data.energy = math.pi, 0.0, 0.0
            key.data.use_shadow = False   # flat cel bands, no cast-shadow noise
            scene.render.use_freestyle = True

    def outline(ob, thickness):
        mod = ob.modifiers.new("Outline", "SOLIDIFY")
        mod.thickness = thickness
        mod.offset = 1.0
        mod.use_flip_normals = True
        mod.use_even_offset = True
        mod.use_quality_normals = True
        mod.material_offset = 1
        mod.use_rim = False

    def group_centre(name):
        gi = avatar.vertex_groups[name].index
        cos = [v.co for v in avatar.data.vertices if any(g.group == gi for g in v.groups)]
        lo = Vector([min(c[i] for c in cos) for i in range(3)])
        hi = Vector([max(c[i] for c in cos) for i in range(3)])
        return (lo + hi) / 2

    def aim(target, ortho, az, el, res):
        if isinstance(target[0], str):
            t = group_centre(target[0]) + Vector(target[1])
        else:
            t = Vector(target)
        azr, elr = math.radians(az), math.radians(el)
        d = Vector((-math.sin(azr) * math.cos(elr), -math.cos(azr) * math.cos(elr), math.sin(elr)))
        cam.location = t + d * 10.0
        if abs(el) > 80:
            # straight up from below; screen up = toward the toe (-Y)
            cam.rotation_euler = (math.pi, 0, 0)
        else:
            cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
        cam_data.ortho_scale = ortho
        scene.render.resolution_x, scene.render.resolution_y = res
        scene.render.resolution_percentage = 100

    # Interior construction lines for the toon pass: Freestyle creases (the elbow, knee, wrist and
    # boot steps, the chest break) plus contours where one part overlaps another. The silhouette
    # itself comes from the inverted hull.
    vl = scene.view_layers[0]
    fs = vl.freestyle_settings
    ls = fs.linesets[0]
    ls.select_by_visibility = True
    ls.visibility = "VISIBLE"
    ls.select_by_edge_types = True
    for attr in ("select_silhouette", "select_border", "select_external_contour", "select_suggestive_contour",
                 "select_ridge_valley", "select_material_boundary"):
        setattr(ls, attr, False)
    ls.select_crease = True
    ls.select_edge_mark = True
    # The modelled plane breaks (edges marked sharp by build_avatar.py) are drawn as construction lines.
    for ob in [avatar] + list(parts.values()):
        me = ob.data
        sharp = me.attributes.get("sharp_edge")
        if sharp is None:
            continue
        flags = [False] * len(me.edges)
        sharp.data.foreach_get("value", flags)
        mark = me.attributes.get("freestyle_edge") or me.attributes.new("freestyle_edge", "BOOLEAN", "EDGE")
        mark.data.foreach_set("value", flags)
    ls.select_contour = True
    fs.crease_angle = math.radians(115)   # only real steps, not the corner bevels
    if ls.linestyle is None:
        ls.linestyle = bpy.data.linestyles.new("Construction")
    style = ls.linestyle
    style.color = tuple(c / 255 for c in TOON_LINE)
    style.alpha = 0.85
    scene.render.line_thickness_mode = "ABSOLUTE"

    os.makedirs(RAW, exist_ok=True)
    names = a.only or list(VIEWS)
    for p in a.passes:
        set_pass(p)
        for name in names:
            groups, target, ortho, az, el, res = VIEWS[name]
            show = parts[groups] if groups else avatar
            for ob in objects:
                ob.hide_render = ob is not show
                for m in [x for x in ob.modifiers if x.type == "SOLIDIFY"]:
                    ob.modifiers.remove(m)
            if p == "toon":
                outline(show, OUTLINE_PX * ortho / max(res))
                style.thickness = INNER_LINE_PX
            aim(target, ortho, az, el, res)
            scene.render.filepath = os.path.join(RAW, f"{p}_{name}.png")
            bpy.ops.render.render(write_still=True)
            print(f"[render] {p}_{name}")
    with open(os.path.join(RAW, "stats.txt"), "w") as f:
        f.write(f"triangles {tris}\n")


# ======================================================================== compose side (Pillow)

def compose_main():
    from PIL import Image, ImageDraw, ImageFont

    os.makedirs(OUT, exist_ok=True)

    def on_bg(path, color):
        im = Image.open(path).convert("RGBA")
        base = Image.new("RGBA", im.size, color + (255,))
        base.alpha_composite(im)
        return base.convert("RGB")

    singles = {}
    for p, color in (("clay", BG_CLAY), ("toon", BG_TOON)):
        for name in VIEWS:
            src = os.path.join(RAW, f"{p}_{name}.png")
            if not os.path.exists(src):
                continue
            im = on_bg(src, color)
            im.save(os.path.join(OUT, f"{p}_{name}.png"), optimize=True)
            singles[(p, name)] = Image.open(src).convert("RGBA")

    font = ImageFont.load_default(size=17)
    font_big = ImageFont.load_default(size=20)
    ink = (110, 80, 70)

    def fit(im, h=None, w=None):
        bb = im.getbbox()
        im = im.crop(bb) if bb else im
        if h:
            s = h / im.height
        else:
            s = w / im.width
        if w and h:
            s = min(h / im.height, w / im.width)
        return im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)

    for p, color in (("clay", BG_CLAY), ("toon", BG_TOON)):
        if (p, "body_front") not in singles:
            continue
        sheet = Image.new("RGBA", (1536, 1024), color + (255,))
        dr = ImageDraw.Draw(sheet)
        # Body views at the concept's figure height (about 624 px), feet on y = 655.
        body = [("body_front", 178, "FRONT"), ("body_front34", 446, "FRONT 3/4"), ("body_side", 670, "SIDE"),
                ("body_back", 892, "BACK"), ("body_back34", 1142, "BACK 3/4")]
        # One scale for all five, taken from the front view's height.
        ref = singles[(p, "body_front")]
        bb = ref.getbbox()
        scale = 624 / (bb[3] - bb[1])
        for name, cx, label in body:
            im = singles.get((p, name))
            if im is None:
                continue
            bb = im.getbbox()
            im = im.crop(bb)
            im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
            sheet.alpha_composite(im, (cx - im.width // 2, 655 - im.height))
            tw = dr.textlength(label, font=font)
            dr.text((cx - tw / 2, 668), label, font=font, fill=ink)
        # Panels.
        def panel(x0, y0, x1, y1, title):
            dr.line([(x0, y0), (x0, y1), (x1, y1)], fill=(150, 140, 130), width=1)
            dr.line([(x0, y0), (x1, y0)], fill=(150, 140, 130), width=1)
            dr.text((x0 + 14, y0 + 8), title, font=font_big, fill=ink)

        panel(1274, 18, 1512, 690, "HEAD")
        for name, cy, label in (("head_front", 160, "FRONT"), ("head_side", 375, "SIDE"), ("head_34", 575, "3/4")):
            im = singles.get((p, name))
            if im is None:
                continue
            im = fit(im, h=168)
            sheet.alpha_composite(im, (1393 - im.width // 2, cy - im.height // 2))
            tw = dr.textlength(label, font=font)
            dr.text((1393 - tw / 2, cy + 85), label, font=font, fill=ink)
        panel(30, 708, 520, 1004, "HANDS")
        for name, cx, label in (("hand_back", 125, "BACK"), ("hand_front", 275, "THUMB SIDE"), ("hand_palm", 425, "PALM")):
            im = singles.get((p, name))
            if im is None:
                continue
            im = fit(im, h=210, w=140)
            sheet.alpha_composite(im, (cx - im.width // 2, 860 - im.height // 2))
            tw = dr.textlength(label, font=font)
            dr.text((cx - tw / 2, 975), label, font=font, fill=ink)
        panel(538, 708, 910, 1004, "FEET")
        for name, cx, label in (("foot_front", 620, "FRONT"), ("foot_side", 735, "SIDE"), ("foot_sole", 850, "SOLE")):
            im = singles.get((p, name))
            if im is None:
                continue
            im = fit(im, h=200, w=110)
            sheet.alpha_composite(im, (cx - im.width // 2, 865 - im.height // 2))
            tw = dr.textlength(label, font=font)
            dr.text((cx - tw / 2, 975), label, font=font, fill=ink)
        panel(928, 708, 1512, 1004, "POSE EXAMPLES")
        msg = "Hand on hip, run and sitting poses come with the rig (Phase B)."
        dr.text((952, 850), msg, font=font, fill=ink)
        sheet.convert("RGB").save(os.path.join(OUT, f"contact_sheet_{p}.png"), optimize=True)
        print("wrote", os.path.join(OUT, f"contact_sheet_{p}.png"))


if __name__ == "__main__":
    if "--compose" in sys.argv:
        compose_main()
    else:
        blender_main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
