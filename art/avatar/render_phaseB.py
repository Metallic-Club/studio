"""Phase B review renders of the rigged default avatar, in grey clay and in the concept's toon look.

Two steps, one file:
  1. Render (inside Blender, EEVEE, low samples), from the .blend that rig_avatar.py saved:
       blender -b --factory-startup --python art/avatar/render_phaseB.py -- SET [SET ...] [--passes clay toon]
     SET is one of: turnaround, poses, deform, strips, emotes (or a clip name for one strip).
     Writes transparent RGBA renders to art/avatar/out/rawB/.
  2. Compose (plain Python 3 with Pillow):
       python art/avatar/render_phaseB.py --compose
     Lays the renders out as review sheets in art/avatar/renders/phaseB/.

Cameras are orthographic and the lights ride on the camera, as in Phase A. Azimuth 0 = in front of
the figure, positive azimuth moves the camera toward the figure's right side.
"""
import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "out", "rawB")
OUT = os.path.join(HERE, "renders", "phaseB")
BLEND = os.path.join(HERE, "out", "default_avatar.blend")
CONCEPT = os.path.join(HERE, "concept", "default-avatar-concept.png")

OUTLINE_PX = 3.0
INNER_LINE_PX = 1.4
BG_CLAY = (222, 222, 222)
BG_TOON = (231, 227, 220)
TOON_BANDS = [(200, 180, 158), (219, 203, 179), (234, 221, 198)]
TOON_LINE = (92, 52, 42)
MARK = (200, 60, 40)
STUD = 0.28

BODY = (520, 1000)
# name: (action, frame, parts or None, target, ortho, azimuth, elevation, resolution)
TURNAROUND = {
    "body_front": ("test_display", 0, None, (0, 0, 0.80), 1.70, 0, 8, BODY),
    "body_front34": ("test_display", 0, None, (0, 0, 0.80), 1.70, 30, 8, BODY),
    "body_side": ("test_display", 0, None, (0, 0, 0.80), 1.70, 90, 8, BODY),
    "body_back": ("test_display", 0, None, (0, 0, 0.80), 1.70, 180, 8, BODY),
    "body_back34": ("test_display", 0, None, (0, 0, 0.80), 1.70, 225, 8, BODY),
    "head_front": ("test_display", 0, ("Head", "Neck"), (0, 0.0, 1.33), 0.50, 0, 4, (480, 480)),
    "head_side": ("test_display", 0, ("Head", "Neck"), (0, 0.0, 1.33), 0.50, -90, 4, (480, 480)),
    "head_34": ("test_display", 0, ("Head", "Neck"), (0, 0.0, 1.33), 0.50, -45, 4, (480, 480)),
    "hand_back": ("test_display", 0, ("Hand.L", "LowerArm.L"), ("hand", "back"), 0.30, 0, 0, (420, 420)),
    "hand_fist": ("test_hand_fist", 0, ("Hand.L", "LowerArm.L"), ("hand", "fist"), 0.30, 0, 0, (420, 420)),
    "hand_open": ("test_hand_open", 0, ("Hand.L", "LowerArm.L"), ("hand", "open"), 0.30, 0, 0, (420, 420)),
    "hand_palm": ("test_hand_open", 0, ("Hand.L", "LowerArm.L"), ("hand", "palm"), 0.30, 0, 0, (420, 420)),
    "foot_front": ("test_rest", 0, ("Boot.R", "Shin.R"), ("Boot.R", (0, 0, 0.075)), 0.40, 0, 20, (420, 420)),
    "foot_side": ("test_rest", 0, ("Boot.R", "Shin.R"), ("Boot.R", (0, 0, 0.075)), 0.40, 90, 10, (420, 420)),
    "foot_sole": ("test_rest", 0, ("Boot.R",), ("Boot.R", (0, 0, 0)), 0.40, 0, -90, (420, 420)),
}

POSE_RES = (520, 520)
POSES = {
    # the concept's pose panel, each seen from about the drawing's angle
    "pose_hand_on_hip": ("pose_hand_on_hip", 0, None, (0, 0, 0.78), 1.75, -18, 6, POSE_RES),
    "pose_run": ("pose_run", 0, None, (0, -0.05, 0.80), 1.75, 80, 6, POSE_RES),
    "pose_sit_back": ("pose_sit_back", 0, None, (0.05, -0.05, 0.42), 1.75, 35, 14, POSE_RES),
}

DEFORM = ["test_knee_bend", "test_arm_overhead", "test_arm_across", "test_wrist_twist", "test_crouch",
          "test_splits"]
DEFORM_VIEWS = {"a": (35, 10), "b": (110, 10)}

# Camera target and ortho size for clips that do not fit the standing frame.
STRIP_FRAME = {"swim": ((0, 0.0, 0.62), 3.4)}
STRIP_CLIPS = ["idle", "walk", "run", "jump", "fall", "land", "idle_b", "climb", "swim", "swim_idle", "sit"]
EMOTE_CLIPS = ["wave", "point", "cheer", "dance"]
STRIP_RES = (300, 460)


def srgb_to_lin(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lin(rgb):
    return tuple(srgb_to_lin(c) for c in rgb) + (1.0,)


# ======================================================================== Blender side

def blender_main(argv):
    import argparse
    import bmesh
    import bpy
    from mathutils import Matrix, Vector

    ap = argparse.ArgumentParser()
    ap.add_argument("sets", nargs="+")
    ap.add_argument("--passes", nargs="*", default=["clay", "toon"])
    ap.add_argument("--samples", type=int, default=12)
    ap.add_argument("--only", nargs="*", default=None)
    a = ap.parse_args(argv)

    bpy.ops.wm.open_mainfile(filepath=BLEND)
    scene = bpy.context.scene
    arm = bpy.data.objects["DefaultAvatarRig"]
    body = bpy.data.objects["DefaultAvatar_LOD0"]
    for o in scene.objects:
        if o.type == "MESH" and o is not body:
            o.hide_render = True
    for eng in ("BLENDER_EEVEE", "BLENDER_EEVEE_NEXT"):
        try:
            scene.render.engine = eng
            break
        except TypeError:
            continue
    scene.eevee.taa_render_samples = a.samples
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.render.fps = 30

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

    def sun(name, rot_deg, angle=2.0):
        d = bpy.data.lights.new(name, "SUN")
        d.angle = math.radians(angle)
        o = bpy.data.objects.new(name, d)
        scene.collection.objects.link(o)
        o.parent = cam
        o.rotation_euler = [math.radians(r) for r in rot_deg]
        return o

    key = sun("Key", (-38, 32, 0))
    fill = sun("Fill", (-10, -55, 0), 8)
    rim = sun("Rim", (-150, -30, 0), 4)

    # Ground helpers for the strips: a thin ground line, ticks every 0.25 m and a foot marker.
    def box(name, size, loc, color):
        me = bpy.data.meshes.new(name)
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        bmesh.ops.scale(bm, vec=size, verts=bm.verts[:])
        bm.to_mesh(me)
        bm.free()
        ob = bpy.data.objects.new(name, me)
        scene.collection.objects.link(ob)
        ob.location = loc
        m = bpy.data.materials.new(name + "_m")
        nt = m.node_tree
        for n in list(nt.nodes):
            nt.nodes.remove(n)
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        em = nt.nodes.new("ShaderNodeEmission")
        em.inputs["Color"].default_value = lin(color)
        nt.links.new(em.outputs[0], out.inputs["Surface"])
        me.materials.append(m)
        ob.hide_render = True
        return ob

    ground = box("Ground", (0.012, 6.0, 0.004), (0, 0, -0.002), (110, 100, 95))
    ticks = [box(f"Tick{i}", (0.012, 0.006, 0.03), (0, 0, 0.0), (110, 100, 95)) for i in range(25)]
    marker = box("FootMarker", (0.012, 0.012, 0.09), (0, 0, 0.0), MARK)

    def part_object(groups):
        ob = body.copy()
        ob.data = body.data.copy()
        ob.name = "Part_" + "_".join(groups)
        scene.collection.objects.link(ob)
        idx = {ob.vertex_groups["part_" + g].index for g in groups}
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

    def part(groups):
        if groups not in parts:
            parts[groups] = part_object(groups)
        return parts[groups]

    clay_mats = list(body.data.materials)

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
    ls.select_contour = True
    fs.crease_angle = math.radians(115)
    if ls.linestyle is None:
        ls.linestyle = bpy.data.linestyles.new("Construction")
    style = ls.linestyle
    style.color = tuple(c / 255 for c in TOON_LINE)
    style.alpha = 0.85
    scene.render.line_thickness_mode = "ABSOLUTE"

    def mark_freestyle(ob):
        me = ob.data
        sharp = me.attributes.get("sharp_edge")
        if sharp is None:
            return
        flags = [False] * len(me.edges)
        sharp.data.foreach_get("value", flags)
        mark = me.attributes.get("freestyle_edge") or me.attributes.new("freestyle_edge", "BOOLEAN", "EDGE")
        mark.data.foreach_set("value", flags)

    mark_freestyle(body)

    def set_pass(p, obs):
        for ob in obs:
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
            key.data.energy, fill.data.energy, rim.data.energy = math.pi, 0.0, 0.0
            key.data.use_shadow = False
            scene.render.use_freestyle = True

    def outline(ob, thickness):
        mod = ob.modifiers.new("Outline", "SOLIDIFY")
        mod.thickness = thickness
        mod.offset = 1.0
        mod.use_flip_normals = True
        mod.use_even_offset = False   # even offset spikes at the fused fist's concave corners
        mod.use_quality_normals = True
        mod.material_offset = 1
        mod.use_rim = False

    def set_action(name, frame):
        act = bpy.data.actions[name]
        ad = arm.animation_data or arm.animation_data_create()
        ad.action = act
        ad.action_slot = act.slots[0]
        scene.frame_set(int(frame))
        bpy.context.view_layer.update()

    def bone_world(name, local=(0, 0, 0)):
        pb = arm.pose.bones[name]
        return arm.matrix_world @ pb.matrix @ Vector(local)

    def group_centre(ob, gname):
        dg = bpy.context.evaluated_depsgraph_get()
        ev = ob.evaluated_get(dg)
        me = ev.to_mesh()
        gi = ob.vertex_groups["part_" + gname].index
        src = ob.data
        cos = [me.vertices[v.index].co for v in src.vertices if any(g.group == gi for g in v.groups)]
        lo = Vector([min(c[i] for c in cos) for i in range(3)])
        hi = Vector([max(c[i] for c in cos) for i in range(3)])
        ev.to_mesh_clear()
        return ob.matrix_world @ ((lo + hi) / 2)

    def aim_dir(t, d, up, ortho, res):
        cam.location = t - d * 10.0
        cam.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()
        # roll so that `up` is screen-up
        q = d.to_track_quat("-Z", "Y")
        cur_up = q @ Vector((0, 1, 0))
        upp = (up - d * up.dot(d)).normalized()
        ang = math.atan2(cur_up.cross(upp).dot(d), cur_up.dot(upp))
        q = Matrix.Rotation(ang, 4, d).to_quaternion() @ q
        cam.rotation_euler = q.to_euler()
        cam_data.ortho_scale = ortho
        scene.render.resolution_x, scene.render.resolution_y = res
        scene.render.resolution_percentage = 100

    def aim(target, ortho, az, el, res, show):
        if isinstance(target[0], str) and target[0] == "hand":
            # Hand close-ups follow the posed hand: a = along the hand, c = palm normal, b = thumb side.
            m = (arm.matrix_world @ arm.pose.bones["leftHand"].matrix).to_3x3()
            av, cv = m @ Vector((0, 1, 0)), m @ Vector((0, 0, 1))
            bv = av.cross(cv).normalized()
            t = bone_world("leftHand", (0, 0.09, 0.0))
            views = {
                "back": (cv * 1.0 + bv * 0.25, -av),
                "fist": ((cv * 0.55 - bv * 0.75 + av * 0.25), -av),
                "open": (cv * 1.0 - bv * 0.15, -av),
                "palm": (-cv * 1.0 - bv * 0.15, -av),
            }
            d, up = views[target[1]]
            aim_dir(t, d.normalized(), up, ortho, res)
            return
        if isinstance(target[0], str):
            t = group_centre(show, target[0]) + Vector(target[1])
        else:
            t = Vector(target)
        azr, elr = math.radians(az), math.radians(el)
        dpos = Vector((-math.sin(azr) * math.cos(elr), -math.cos(azr) * math.cos(elr), math.sin(elr)))
        cam.location = t + dpos * 10.0
        if abs(el) > 80:
            cam.rotation_euler = (math.pi, 0, 0)
        else:
            cam.rotation_euler = (-dpos).to_track_quat("-Z", "Y").to_euler()
        cam_data.ortho_scale = ortho
        scene.render.resolution_x, scene.render.resolution_y = res
        scene.render.resolution_percentage = 100

    def render(out_name, p, action, frame, groups, target, ortho, az, el, res, extras=()):
        set_action(action, frame)
        show = part(groups) if groups else body
        for ob in [body] + list(parts.values()):
            ob.hide_render = ob is not show
        for ob in [ground, marker] + ticks:
            ob.hide_render = ob not in extras
        set_pass(p, [show])
        if p == "toon":
            outline(show, OUTLINE_PX * ortho / max(res))
            style.thickness = INNER_LINE_PX
        if groups:
            mark_freestyle(show)
        aim(target, ortho, az, el, res, show)
        scene.render.filepath = os.path.join(RAW, f"{p}_{out_name}.png")
        bpy.ops.render.render(write_still=True)
        print(f"[render] {p}_{out_name}")

    os.makedirs(RAW, exist_ok=True)
    meta = {}

    def strip(clip, p, views):
        act = bpy.data.actions.get(clip)
        if act is None:
            print("[skip]", clip)
            return
        n_last = int(act.frame_range[1])
        loop = bool(act.get("loop", False))
        speed = float(act.get("speed", 0.0))
        n = next((k for k in range(12, 7, -1) if n_last % k == 0), 12) if loop else 12
        n = min(n, n_last + 1)
        frames = [round(i * n_last / n) for i in range(n)] if loop else \
            [round(i * n_last / (n - 1)) for i in range(n)]
        frames = sorted(set(frames))
        extras = (ground,) + tuple(ticks) + ((marker,) if speed else ())
        # The marker sits where the left heel lands on frame 0 and moves with the ground.
        y0 = None
        if speed:
            set_action(clip, 0)
            y0 = bone_world("leftFoot").y + 0.112
        for vname, (az, el) in views.items():
            for f in frames:
                t = f / 30.0
                shift = speed * t
                for i, tk in enumerate(ticks):
                    tk.location = (0, ((i - 12) * 0.25 + shift) % 6.0 - 3.0, 0.015)
                if speed:
                    marker.location = (0.15 if az > 0 else -0.15, y0 + shift, 0.045)
                set_action(clip, f)
                tgt, ortho = STRIP_FRAME.get(clip, ((0, 0, 0.78), 2.0))
                render(f"strip_{clip}_{vname}_{f:03d}", p, clip, f, None, tgt, ortho, az, el,
                       STRIP_RES, extras)
        meta[clip] = {"frames": frames, "last": n_last, "loop": loop, "speed": speed, "views": list(views)}

    for p in a.passes:
        for s in a.sets:
            if s == "turnaround":
                for name, spec in TURNAROUND.items():
                    if a.only and name not in a.only:
                        continue
                    render(name, p, *spec)
            elif s == "poses":
                for name, spec in POSES.items():
                    if a.only and name not in a.only:
                        continue
                    render(name, p, *spec)
            elif s == "deform":
                for clip in DEFORM:
                    if a.only and clip not in a.only:
                        continue
                    for vn, (az, el) in DEFORM_VIEWS.items():
                        render(f"{clip}_{vn}", p, clip, 0, None, (0, 0, 0.72), 1.9, az, el, (420, 520))
            elif s in ("strips", "emotes") or s in STRIP_CLIPS + EMOTE_CLIPS:
                names = STRIP_CLIPS if s == "strips" else (EMOTE_CLIPS if s == "emotes" else [s])
                for clip in names:
                    if a.only and clip not in a.only:
                        continue
                    views = {"side": (90, 4)}
                    if clip in ("walk", "run"):
                        views["front"] = (20, 6)
                    strip(clip, p, views)
    if meta:
        path = os.path.join(RAW, "strips.json")
        old = json.load(open(path)) if os.path.exists(path) else {}
        old.update(meta)
        json.dump(old, open(path, "w"), indent=1)


# ======================================================================== compose side (Pillow)

def compose_main(argv):
    from PIL import Image, ImageDraw, ImageFont

    os.makedirs(OUT, exist_ok=True)
    font = ImageFont.load_default(size=16)
    font_big = ImageFont.load_default(size=20)
    ink = (110, 80, 70)

    def raw(p, name):
        path = os.path.join(RAW, f"{p}_{name}.png")
        return Image.open(path).convert("RGBA") if os.path.exists(path) else None

    def crop(im):
        bb = im.getbbox()
        return im.crop(bb) if bb else im

    def fit(im, w, h):
        im = crop(im)
        s = min(w / im.width, h / im.height)
        return im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)

    def label(dr, x, y, text, f=font, center=True):
        tw = dr.textlength(text, font=f)
        dr.text((x - tw / 2 if center else x, y), text, font=f, fill=ink)

    concept = Image.open(CONCEPT).convert("RGBA")

    for p, bgc in (("clay", BG_CLAY), ("toon", BG_TOON)):
        # 1. Turnaround sheet in the concept's layout.
        if raw(p, "body_front"):
            sheet = Image.new("RGBA", (1536, 1024), bgc + (255,))
            dr = ImageDraw.Draw(sheet)
            ref = raw(p, "body_front")
            bb = ref.getbbox()
            scale = 624 / (bb[3] - bb[1])
            for name, cx, lab in [("body_front", 178, "FRONT"), ("body_front34", 446, "FRONT 3/4"),
                                  ("body_side", 670, "SIDE"), ("body_back", 892, "BACK"),
                                  ("body_back34", 1142, "BACK 3/4")]:
                im = raw(p, name)
                if im is None:
                    continue
                im = crop(im)
                im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
                sheet.alpha_composite(im, (cx - im.width // 2, 655 - im.height))
                label(dr, cx, 668, lab)

            def panel(x0, y0, x1, y1, title):
                dr.line([(x0, y0), (x0, y1), (x1, y1)], fill=(150, 140, 130), width=1)
                dr.line([(x0, y0), (x1, y0)], fill=(150, 140, 130), width=1)
                dr.text((x0 + 14, y0 + 8), title, font=font_big, fill=ink)

            panel(1274, 18, 1512, 690, "HEAD")
            for name, cy, lab in (("head_front", 160, "FRONT"), ("head_side", 375, "SIDE"), ("head_34", 575, "3/4")):
                im = raw(p, name)
                if im:
                    im = fit(im, 200, 168)
                    sheet.alpha_composite(im, (1393 - im.width // 2, cy - im.height // 2))
                    label(dr, 1393, cy + 85, lab)
            panel(30, 708, 520, 1004, "HANDS")
            for name, cx, lab in (("hand_back", 120, "BACK"), ("hand_fist", 275, "FIST"), ("hand_open", 430, "OPEN")):
                im = raw(p, name)
                if im:
                    im = fit(im, 145, 215)
                    sheet.alpha_composite(im, (cx - im.width // 2, 860 - im.height // 2))
                    label(dr, cx, 975, lab)
            panel(538, 708, 910, 1004, "FEET")
            for name, cx, lab in (("foot_front", 620, "FRONT"), ("foot_side", 735, "SIDE"), ("foot_sole", 850, "SOLE")):
                im = raw(p, name)
                if im:
                    im = fit(im, 110, 200)
                    sheet.alpha_composite(im, (cx - im.width // 2, 865 - im.height // 2))
                    label(dr, cx, 975, lab)
            panel(928, 708, 1512, 1004, "POSE EXAMPLES")
            for name, cx in (("pose_hand_on_hip", 1035), ("pose_run", 1215), ("pose_sit_back", 1395)):
                im = raw(p, name)
                if im:
                    im = fit(im, 175, 250)
                    sheet.alpha_composite(im, (cx - im.width // 2, 990 - im.height))
            sheet.convert("RGB").save(os.path.join(OUT, f"contact_sheet_{p}.png"), optimize=True)

        # 2. Poses next to the concept's pose panel.
        if raw(p, "pose_run"):
            sheet = Image.new("RGBA", (1500, 900), bgc + (255,))
            dr = ImageDraw.Draw(sheet)
            cpanel = concept.crop((925, 735, 1520, 1010))
            cpanel = cpanel.resize((1190, 550), Image.LANCZOS)
            sheet.alpha_composite(cpanel, (155, 10))
            label(dr, 750, 565, "CONCEPT (top)  /  MODEL (bottom)")
            for name, cx in (("pose_hand_on_hip", 330), ("pose_sit_back", 1170), ("pose_run", 750)):
                im = raw(p, name)
                if im:
                    im = fit(im, 380, 300)
                    sheet.alpha_composite(im, (cx - im.width // 2, 890 - im.height))
            sheet.convert("RGB").save(os.path.join(OUT, f"poses_vs_concept_{p}.png"), optimize=True)

        # 3. Deformation test sheet.
        if raw(p, "test_knee_bend_a"):
            cw, ch = 240, 300
            sheet = Image.new("RGBA", (cw * 6 + 40, ch * 2 + 90), bgc + (255,))
            dr = ImageDraw.Draw(sheet)
            for i, name in enumerate(DEFORM):
                label(dr, 20 + cw * i + cw // 2, 8, name.replace("test_", "").replace("_", " ").upper())
                for j, vn in enumerate(("a", "b")):
                    im = raw(p, f"{name}_{vn}")
                    if im:
                        im = fit(im, cw - 16, ch - 16)
                        sheet.alpha_composite(im, (20 + cw * i + (cw - im.width) // 2, 34 + ch * j + (ch - im.height)))
            label(dr, 20, ch * 2 + 50, "Deformation tests: top row 3/4 front-right, bottom row side-back.", center=False)
            sheet.convert("RGB").save(os.path.join(OUT, f"deformation_{p}.png"), optimize=True)

        # 4. Clip strips.
        mpath = os.path.join(RAW, "strips.json")
        meta = json.load(open(mpath)) if os.path.exists(mpath) else {}
        for clip, m in meta.items():
            for vname in m["views"]:
                ims = [raw(p, f"strip_{clip}_{vname}_{f:03d}") for f in m["frames"]]
                if not all(ims):
                    continue
                w, h = ims[0].size
                cw = int(w * 0.84)
                sheet = Image.new("RGBA", (cw * len(ims) + 20, h + 60), bgc + (255,))
                dr = ImageDraw.Draw(sheet)
                for i, (im, f) in enumerate(zip(ims, m["frames"])):
                    tile = im.crop(((w - cw) // 2, 0, (w + cw) // 2, h))
                    sheet.alpha_composite(tile, (10 + cw * i, 34))
                    label(dr, 10 + cw * i + cw // 2, h + 36, f"f{f}")
                title = f"{clip.upper()}  ({vname}, frames 0-{m['last']}, {'loop' if m['loop'] else 'one-shot'}"
                if m["speed"]:
                    title += f", {m['speed']:.2f} m/s = {m['speed'] / STUD:.0f} studs/s; red post = fixed ground point"
                title += ")"
                dr.text((12, 8), title, font=font, fill=ink)
                sheet.convert("RGB").save(os.path.join(OUT, f"strip_{clip}_{vname}_{p}.png"), optimize=True)
    print("composed into", OUT)


if __name__ == "__main__":
    if "--compose" in sys.argv:
        compose_main(sys.argv)
    else:
        blender_main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
