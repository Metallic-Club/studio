"""Animation review strips for the v3 clips: side and front views of each clip, frame by frame.

Two steps (like render_phaseB.py, but Workbench for speed so every critique round is cheap):
  1. Render, inside Blender, from the .blend that rig_avatar.py saved:
       blender -b --factory-startup --python art/avatar/render_anim_review.py -- CLIP [CLIP ...] [--frames N]
     Writes raw PNGs and a manifest to art/avatar/out/rawV3/.
  2. Compose (plain Python 3 with Pillow):
       python art/avatar/render_anim_review.py --compose [--out DIR]
     One sheet per clip, side view on top and a front three-quarter view below, into
     art/avatar/renders/v3-anim/ (or DIR).

The ground ticks are 0.25 m apart and move with the clip's authored speed, and a red post marks
where the left heel lands on frame 0: on a gait a planted foot must stay on its post. A grey
silhouette of frame 0 sits behind every panel of a loop so pops and drifts show.
"""
import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "out", "rawV3")
OUT = os.path.join(HERE, "renders", "v3-anim")
BLEND = os.path.join(HERE, "out", "default_avatar.blend")
PANEL = (250, 380)
WIDE = {"swim": ((0, 0.0, 0.55), 2.9), "swim_idle": ((0, 0, 0.70), 2.2), "climb": ((0, 0, 0.85), 2.2),
        "fall": ((0, 0, 0.85), 2.2), "jump": ((0, 0, 0.85), 2.2), "dance": ((0, 0, 0.80), 2.2),
        "cheer": ((0, 0, 0.85), 2.2), "wave": ((0, 0, 0.80), 2.1)}
FRONT_CLIPS = None   # every clip gets both views
# Clips also seen from the gameplay camera (behind the player): a back and a rear 3/4 row.
GAMEPLAY_VIEWS = {"jump", "fall", "land", "land_hard"}


def blender_main(argv):
    import argparse
    import bmesh
    import bpy
    from mathutils import Vector

    ap = argparse.ArgumentParser()
    ap.add_argument("clips", nargs="+")
    ap.add_argument("--frames", type=int, default=10)
    a = ap.parse_args(argv)

    bpy.ops.wm.open_mainfile(filepath=BLEND)
    scene = bpy.context.scene
    arm = bpy.data.objects["DefaultAvatarRig"]
    body = bpy.data.objects["DefaultAvatar_LOD0"]
    for o in scene.objects:
        if o.type == "MESH" and o is not body:
            o.hide_render = True
    scene.render.engine = "BLENDER_WORKBENCH"
    sh = scene.display.shading
    sh.light = "STUDIO"
    sh.color_type = "SINGLE"
    sh.single_color = (0.78, 0.76, 0.74)
    sh.show_cavity = True
    sh.cavity_type = "BOTH"
    sh.show_object_outline = True
    sh.object_outline_color = (0.25, 0.16, 0.13)
    sh.show_shadows = False
    scene.display.render_aa = "8"
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.resolution_x, scene.render.resolution_y = PANEL
    scene.render.resolution_percentage = 100
    scene.render.fps = 30
    scene.view_settings.view_transform = "Standard"

    cam_data = bpy.data.cameras.new("Cam")
    cam_data.type = "ORTHO"
    cam_data.clip_start, cam_data.clip_end = 0.01, 50
    cam = bpy.data.objects.new("Cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam

    def box(name, size, color):
        me = bpy.data.meshes.new(name)
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        bmesh.ops.scale(bm, vec=size, verts=bm.verts[:])
        bm.to_mesh(me)
        bm.free()
        ob = bpy.data.objects.new(name, me)
        ob.color = color + (1.0,)
        scene.collection.objects.link(ob)
        return ob

    sh.color_type = "OBJECT"
    body.color = (0.78, 0.76, 0.74, 1.0)
    ground = box("Ground", (0.02, 8.0, 0.006), (0.35, 0.32, 0.30))
    ground.location = (0, 0, -0.003)
    ticks = [box(f"Tick{i}", (0.02, 0.008, 0.035), (0.35, 0.32, 0.30)) for i in range(33)]
    marker = box("Marker", (0.02, 0.014, 0.10), (0.85, 0.20, 0.12))

    pb = arm.pose.bones

    def set_action(name, frame):
        act = bpy.data.actions[name]
        ad = arm.animation_data or arm.animation_data_create()
        ad.action = act
        if act.slots:
            ad.action_slot = act.slots[0]
        scene.frame_set(int(frame))

    def aim(target, ortho, az, el):
        t = Vector(target)
        azr, elr = math.radians(az), math.radians(el)
        d = Vector((math.sin(azr) * math.cos(elr), -math.cos(azr) * math.cos(elr), math.sin(elr)))
        cam.location = t + d * 10.0
        cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
        cam_data.ortho_scale = ortho

    os.makedirs(RAW, exist_ok=True)
    manifest_path = os.path.join(RAW, "manifest.json")
    manifest = json.load(open(manifest_path)) if os.path.exists(manifest_path) else {}
    for clip in a.clips:
        views = {"side": (90, 3), "front": (25, 8)}
        if clip in GAMEPLAY_VIEWS:
            views.update({"back": (180, 10), "rear34": (215, 14)})
        act = bpy.data.actions.get(clip)
        if act is None:
            print("[skip]", clip)
            continue
        last = int(round(act.frame_range[1]))
        loop = bool(act.get("loop", False))
        speed = float(act.get("speed", 0.0))
        n = min(a.frames, last + 1) if last > 0 else 1
        frames = [round(i * last / n) for i in range(n)] if loop else [round(i * last / max(1, n - 1)) for i in range(n)]
        frames = sorted(set(frames))
        y0 = None
        if speed:
            set_action(clip, 0)
            heel = arm.matrix_world @ pb["leftFoot"].head
            y0 = heel.y + 0.11
        tgt, ortho = WIDE.get(clip, ((0, 0, 0.80), 1.75))
        for vname, (az, el) in views.items():
            for f in frames:
                shift = speed * f / 30.0
                for i, tk in enumerate(ticks):
                    tk.location = (0, ((i - 16) * 0.25 + shift) % 8.0 - 4.0, 0.016)
                marker.hide_render = not speed
                if speed:
                    marker.location = (0.16 if az > 0 else -0.16, y0 + shift, 0.05)
                set_action(clip, f)
                aim(tgt, ortho, az, el)
                scene.render.filepath = os.path.join(RAW, f"{clip}_{vname}_{f:03d}.png")
                bpy.ops.render.render(write_still=True)
        manifest[clip] = {"frames": frames, "last": last, "loop": loop, "speed": speed,
                          "views": list(views), "source": act.get("source", "")}
        print(f"[review] {clip}: {frames}")
    json.dump(manifest, open(manifest_path, "w"), indent=1)


def compose_main(argv):
    from PIL import Image, ImageDraw, ImageFont
    out = OUT
    if "--out" in argv:
        out = argv[argv.index("--out") + 1]
    os.makedirs(out, exist_ok=True)
    man = json.load(open(os.path.join(RAW, "manifest.json")))
    only = [x for x in argv if not x.startswith("--") and x not in (out,)]
    try:
        font = ImageFont.truetype("arial.ttf", 15)
        small = ImageFont.truetype("arial.ttf", 13)
    except OSError:
        font = small = ImageFont.load_default()
    bg = (226, 224, 220)
    for clip, m in man.items():
        if only and clip not in only:
            continue
        fr = m["frames"]
        W, H = PANEL
        head = 30
        sheet = Image.new("RGB", (W * len(fr), head + H * len(m["views"]) + 4), bg)
        dr = ImageDraw.Draw(sheet)
        kind = "loop" if m["loop"] else "one-shot"
        sp = f", {m['speed']:.2f} m/s = {m['speed'] / 0.28:.0f} studs/s, red post = fixed ground point" if m["speed"] else ""
        src = f"  [{m['source']}]" if m.get("source") else ""
        dr.text((8, 7), f"{clip.upper()}  frames 0-{m['last']} at 30 fps, {kind}{sp}{src}", fill=(70, 50, 40), font=font)
        for r, v in enumerate(m["views"]):
            ghost = None
            for c, f in enumerate(fr):
                p = os.path.join(RAW, f"{clip}_{v}_{f:03d}.png")
                if not os.path.exists(p):
                    continue
                im = Image.open(p).convert("RGBA")
                if False:
                    ghost = im.copy()
                    alpha = ghost.split()[3].point(lambda x: x * 0.10)
                    g = Image.new("RGBA", im.size, (60, 90, 170, 0))
                    g.putalpha(alpha)
                    ghost = g
                x, y = c * W, head + r * H
                if ghost is not None and c > 0:
                    sheet.paste(ghost, (x, y), ghost)
                sheet.paste(im, (x, y), im)
                dr.text((x + 6, y + H - 18), f"f{f}  {v}", fill=(90, 70, 60), font=small)
        # Grey clay and a few label colours: a 64-colour palette keeps the committed strips small.
        sheet.quantize(colors=64, method=Image.Quantize.MEDIANCUT).save(os.path.join(out, f"{clip}.png"), optimize=True)
    print("composed into", out)


if __name__ == "__main__":
    if "--compose" in sys.argv:
        compose_main(sys.argv[sys.argv.index("--compose") + 1:])
    else:
        blender_main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
