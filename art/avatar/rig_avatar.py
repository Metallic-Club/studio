"""Rig, skin, animate and export Metallic's default avatar (Phase B).

Run headless (one Blender process, about a minute):
    blender -b --factory-startup --python art/avatar/rig_avatar.py -- [--no-export] [--only CLIP ...]

What it does, all from nothing and deterministic:
1. builds LOD0, LOD1 and LOD2 with build_avatar.py;
2. builds one skeleton: a root bone plus VRM-humanoid bones (thumb and four fingers with 3 bones each);
3. skins every LOD to it: rigid weights per part, a short blend only where one shell spans a joint
   (torso, fingers, thumb, boot toes), at most 4 influences per vertex;
4. adds an IK control rig for authoring (foot and hand IK with poles, an FK/IK switch per limb);
5. bakes every clip from anim_clips.py to FK keys on the deform bones, one action per clip;
6. exports art/avatar/out/default_avatar.glb (three skinned meshes, one skeleton, every clip) plus
   default_avatar_clips.json, then re-imports the glb into an empty scene and checks it;
7. saves art/avatar/out/default_avatar.blend for authoring and the review renders.

Bone axes (every deform bone): Y runs along the bone. X is the hinge axis, so a positive X
rotation is flexion: spine, neck and head bend forward; upperLeg swings the thigh forward;
lowerLeg is the exception, its knee flexion is negative X; upperArm swings forward, lowerArm bends
the elbow, hand flexes the wrist toward the palm, fingers and thumb curl into the palm; foot and
toes lift the toes. Y twists along the bone, Z swings sideways. Left and right are exact mirror
images, so a mirrored pose keeps X and negates Y and Z.
"""
import argparse
import json
import math
import os
import sys

import bpy
from mathutils import Euler, Matrix, Quaternion, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import build_avatar as BA  # noqa: E402

OUT = os.path.join(HERE, "out")
FPS = 30
FRONT = Vector((0, -1, 0))
UP = Vector((0, 0, 1))

SIDES = (("left", 1.0), ("right", -1.0))
FINGER_NAMES = ("Index", "Middle", "Ring", "Little")


def mx(v, sign):
    return Vector((v.x * sign, v.y, v.z))


# ======================================================================== skeleton

def bone_table():
    """name -> (head, tail, z_axis_target, parent, deform). Left side mirrored for right."""
    sk = BA.skeleton()
    a, b, c = BA.hand_frame()
    t = {
        "root": (Vector((0, 0, 0)), Vector((0, -0.25, 0)), UP, None),
        "hips": (Vector((0, 0.0, 0.760)), Vector((0, 0.0, 0.845)), FRONT, "root"),
        "spine": (Vector((0, 0.004, 0.845)), Vector((0, 0.004, 0.935)), FRONT, "hips"),
        "chest": (Vector((0, 0.004, 0.935)), Vector((0, 0.006, 1.020)), FRONT, "spine"),
        "upperChest": (Vector((0, 0.006, 1.020)), Vector((0, 0.010, 1.100)), FRONT, "chest"),
        "neck": (Vector((0, 0.010, 1.100)), Vector((0, 0.010, 1.190)), FRONT, "upperChest"),
        "head": (Vector((0, 0.010, 1.190)), Vector((0, 0.010, 1.420)), FRONT, "neck"),
    }
    for side, s in SIDES:
        P = side
        t[P + "Shoulder"] = (mx(Vector((0.022, 0.004, 1.072)), s), mx(sk["SHOULDER"], s), UP, "upperChest")
        t[P + "UpperArm"] = (mx(sk["SHOULDER"], s), mx(sk["ELBOW"], s), FRONT, P + "Shoulder")
        t[P + "LowerArm"] = (mx(sk["ELBOW"], s), mx(sk["WRIST"], s), FRONT, P + "UpperArm")
        t[P + "Hand"] = (mx(sk["WRIST"], s), mx(sk["HAND_END"], s), mx(c, s), P + "LowerArm")
        th = sk["THUMB"]
        for i, part in enumerate(("Metacarpal", "Proximal", "Distal")):
            t[P + "Thumb" + part] = (mx(th[i], s), mx(th[i + 1], s), mx(c, s),
                                     P + ("Hand" if i == 0 else "Thumb" + ("Metacarpal", "Proximal")[i - 1]))
        for f in FINGER_NAMES:
            fp = sk[f.upper()]
            for i, part in enumerate(("Proximal", "Intermediate", "Distal")):
                t[P + f + part] = (mx(fp[i], s), mx(fp[i + 1], s), mx(c, s),
                                   P + ("Hand" if i == 0 else f + ("Proximal", "Intermediate")[i - 1]))
        t[P + "UpperLeg"] = (mx(sk["HIP"], s), mx(sk["KNEE"], s), FRONT, "hips")
        t[P + "LowerLeg"] = (mx(sk["KNEE"], s), mx(sk["ANKLE"], s), FRONT, P + "UpperLeg")
        t[P + "Foot"] = (mx(sk["ANKLE"], s), mx(sk["BALL"], s), UP, P + "LowerLeg")
        t[P + "Toes"] = (mx(sk["BALL"], s), mx(sk["TOE"], s), UP, P + "Foot")
    return t


def build_armature():
    arm_data = bpy.data.armatures.new("DefaultAvatarRig")
    arm = bpy.data.objects.new("DefaultAvatarRig", arm_data)
    bpy.context.scene.collection.objects.link(arm)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    eb = arm_data.edit_bones
    table = bone_table()
    for name, (h, tl, zt, parent) in table.items():
        b = eb.new(name)
        b.head, b.tail = h, tl
        b.align_roll(zt)
        b.use_deform = True
    for name, (h, tl, zt, parent) in table.items():
        if parent:
            eb[name].parent = eb[parent]
            eb[name].use_connect = False
    # IK controls (not exported). The foot and hand controls copy their deform bone exactly, so the
    # rest pose is unchanged when the IK is switched on with the controls at rest.
    sk = BA.skeleton()
    for side, s in SIDES:
        for ctrl, src in (("ik_foot", "Foot"), ("ik_hand", "Hand")):
            b = eb.new(f"{ctrl}.{side}")
            ref = eb[side + src]
            b.head, b.tail, b.roll = ref.head.copy(), ref.tail.copy(), ref.roll
            b.parent, b.use_deform = eb["root"], False
        for ctrl, joint, off in (("pole_knee", "KNEE", Vector((0, -0.45, 0.05))),
                                 ("pole_elbow", "ELBOW", Vector((0, 0.40, 0.0)))):
            b = eb.new(f"{ctrl}.{side}")
            p = mx(sk[joint], s) + off
            b.head, b.tail = p, p + Vector((0, 0, 0.06))
            b.parent, b.use_deform = eb["root"], False
    bpy.ops.object.mode_set(mode="OBJECT")
    return arm


def ik_chain_specs():
    return [(f"leg.{side}", side + "LowerLeg", side + "Foot", f"ik_foot.{side}", f"pole_knee.{side}")
            for side, _ in SIDES] + \
           [(f"arm.{side}", side + "LowerArm", side + "Hand", f"ik_hand.{side}", f"pole_elbow.{side}")
            for side, _ in SIDES]


def add_ik(arm):
    """IK on the lower leg / lower arm (chain of 2) with a pole, plus Copy Rotation on the foot /
    hand. One custom property per limb (0 = FK, 1 = IK) drives both constraints."""
    pb = arm.pose.bones
    for key, low, end, tgt, pole in ik_chain_specs():
        arm[f"ik_{key}"] = 0.0
        ik = pb[low].constraints.new("IK")
        ik.name = "IK"
        ik.target, ik.subtarget = arm, tgt
        ik.pole_target, ik.pole_subtarget = arm, pole
        ik.chain_count = 2
        ik.use_tail = True
        ik.use_stretch = False
        cr = pb[end].constraints.new("COPY_ROTATION")
        cr.name = "IK_rot"
        cr.target, cr.subtarget = arm, tgt
        for con in (ik, cr):
            d = con.driver_add("influence").driver
            d.type = "SCRIPTED"
            var = d.variables.new()
            var.name = "v"
            var.targets[0].id = arm
            var.targets[0].data_path = f'["ik_{key}"]'
            d.expression = "v"
    for p in pb:
        p.rotation_mode = "QUATERNION"
    # Pole angle: the angle at which the IK keeps the rest pose with the controls at rest.
    vl = bpy.context.view_layer
    for key, low, end, tgt, pole in ik_chain_specs():
        upper = pb[low].parent
        arm[f"ik_{key}"] = 1.0
        ik = pb[low].constraints["IK"]

        def err(angle):
            ik.pole_angle = math.radians(angle)
            arm.update_tag()
            vl.update()
            e = 0.0
            for bb in (upper, pb[low]):
                d = bb.matrix.to_3x3() - bb.bone.matrix_local.to_3x3()
                e += sum(d[i][j] ** 2 for i in range(3) for j in range(3))
            return e

        best = min(range(-180, 180, 5), key=err)
        fine = min([best + d * 0.25 for d in range(-20, 21)], key=err)
        e = err(fine)
        print(f"[rig] pole angle {key}: {fine:.2f} deg (rest error {e:.2e})")
        arm[f"ik_{key}"] = 0.0
    vl.update()


# ======================================================================== weights

def smooth01(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


def ramp(s, j, k):
    """0 before j - k, 0.5 at j, 1 after j + k (linear: the joint ring is the only blended ring)."""
    return max(0.0, min(1.0, (s - j) / (2 * k) + 0.5))


def chain_weights(s, joints, bones, k):
    """Weights along a chain: bones[i] owns the span between joints; a short blend at each joint."""
    w = {bones[0]: 1.0}
    for j, (pos, b) in enumerate(zip(joints, bones[1:])):
        t = ramp(s, pos, k)
        if t <= 0:
            break
        prev = bones[j]
        w[prev] = w.get(prev, 0) * (1 - t)
        w[b] = t
    return {b: x for b, x in w.items() if x > 1e-4}


TORSO_Z = [(0.800, {"hips": 1.0}), (0.845, {"hips": 0.5, "spine": 0.5}), (0.890, {"spine": 1.0}),
           (0.935, {"spine": 0.5, "chest": 0.5}), (0.990, {"chest": 1.0}),
           (1.035, {"chest": 0.3, "upperChest": 0.7}), (1.062, {"upperChest": 1.0})]


def torso_weights(co):
    z = co.z
    if z <= TORSO_Z[0][0]:
        w = dict(TORSO_Z[0][1])
    elif z >= TORSO_Z[-1][0]:
        w = dict(TORSO_Z[-1][1])
    else:
        for (z0, w0), (z1, w1) in zip(TORSO_Z, TORSO_Z[1:]):
            if z0 <= z <= z1:
                t = (z - z0) / (z1 - z0)
                w = {}
                for b in set(w0) | set(w1):
                    w[b] = w0.get(b, 0) * (1 - t) + w1.get(b, 0) * t
                break
    # The shoulder tops ride on the shoulder bones (shrug, arm raised).
    sh = smooth01((abs(co.x) - 0.075) / 0.050) * smooth01((z - 0.985) / 0.045) * 0.9
    if sh > 0:
        w = {b: x * (1 - sh) for b, x in w.items()}
        w[("left" if co.x > 0 else "right") + "Shoulder"] = sh
    return w


def vertex_weights(part, co, sk):
    """part = the shell's vertex group name, co = rest position."""
    side = "left" if co.x >= 0 else "right"
    s = 1.0 if co.x >= 0 else -1.0
    p = Vector((co.x * s, co.y, co.z))          # in left-side coordinates
    base = part[:-2] if part[-2:] in (".L", ".R") else part
    if base == "Head":
        return {"head": 1.0}
    if base == "Neck":
        return {"neck": 1.0}
    if base == "Pelvis":
        return {"hips": 1.0}
    if base == "Torso":
        return torso_weights(co)
    if base == "UpperArm":
        return {side + "UpperArm": 1.0}
    if base == "LowerArm":
        return {side + "LowerArm": 1.0}
    if base == "Palm":
        return {side + "Hand": 1.0}
    if base == "Thigh":
        return {side + "UpperLeg": 1.0}
    if base == "Shin":
        return {side + "LowerLeg": 1.0}
    if base == "Boot":
        ly = BA.BOOT_CY - p.y
        t = ramp(ly, BA.BALL_LY, 0.012)
        w = {side + "Foot": 1 - t, side + "Toes": t}
        return {b: x for b, x in w.items() if x > 1e-4}
    if base in FINGER_NAMES:
        fp = sk[base.upper()]
        d = (fp[1] - fp[0]).normalized()
        sd = (p - fp[0]).dot(d)
        joints = [0.0, (fp[1] - fp[0]).length, (fp[2] - fp[0]).length]
        bones = [side + "Hand"] + [side + base + x for x in ("Proximal", "Intermediate", "Distal")]
        return chain_weights(sd, joints, bones, 0.003)
    if base == "Thumb":
        q = [sk["THUMB"][0] - (sk["THUMB"][1] - sk["THUMB"][0]).normalized() * 0.02] + list(sk["THUMB"])
        # arc length along the thumb polyline of the closest point
        best, arc, acc = None, 0.0, 0.0
        for i in range(len(q) - 1):
            seg = q[i + 1] - q[i]
            L = seg.length
            t = max(0.0, min(1.0, (p - q[i]).dot(seg) / (L * L)))
            dist = (q[i] + seg * t - p).length
            if best is None or dist < best:
                best, arc = dist, acc + t * L
            acc += L
        j0 = (q[1] - q[0]).length
        joints = [j0, j0 + (q[2] - q[1]).length, j0 + (q[2] - q[1]).length + (q[3] - q[2]).length]
        bones = [side + "Hand", side + "ThumbMetacarpal", side + "ThumbProximal", side + "ThumbDistal"]
        return chain_weights(arc, joints, bones, 0.0035)
    raise KeyError(part)


SHELL_PARTS = {"Head", "Neck", "Torso", "Pelvis", "UpperArm", "LowerArm", "Palm", "Thumb", "Thigh", "Shin",
               "Boot"} | set(FINGER_NAMES)


def skin(mesh_ob, arm):
    sk = BA.skeleton()
    me = mesh_ob.data
    shell_groups = {g.index: g.name for g in mesh_ob.vertex_groups
                    if (g.name[:-2] if g.name[-2:] in (".L", ".R") else g.name) in SHELL_PARTS}
    for bone in arm.data.bones:
        if bone.use_deform and bone.name not in mesh_ob.vertex_groups:
            mesh_ob.vertex_groups.new(name=bone.name)
    vg = mesh_ob.vertex_groups
    max_inf = 0
    for v in me.vertices:
        part = next(shell_groups[g.group] for g in v.groups if g.group in shell_groups)
        w = vertex_weights(part, v.co, sk)
        w = dict(sorted(w.items(), key=lambda kv: -kv[1])[:4])
        tot = sum(w.values())
        for b, x in w.items():
            vg[b].add([v.index], x / tot, "REPLACE")
        max_inf = max(max_inf, len(w))
    # Drop the part groups so only bone groups remain (the exporter ignores the rest anyway, but
    # it keeps the skin clean). Keep them for the renders' close-ups under a prefix.
    for g in list(mesh_ob.vertex_groups):
        if g.name not in arm.data.bones:
            g.name = "part_" + g.name
    mod = mesh_ob.modifiers.new("Armature", "ARMATURE")
    mod.object = arm
    mesh_ob.parent = arm
    return max_inf


# ======================================================================== pose evaluation and baking

def deform_bones(arm):
    return [b for b in arm.data.bones if b.use_deform]


def reset_pose(arm):
    for p in arm.pose.bones:
        p.location = (0, 0, 0)
        p.rotation_quaternion = (1, 0, 0, 0)
        p.scale = (1, 1, 1)


def apply_pose(arm, pose):
    """pose: dict with 'rot' {bone: (x, y, z) degrees, XYZ euler in the bone's local frame},
    'hips' (dx, dy, dz) world offset of the hips, 'ik' {chain: (position, 3x3 world rotation,
    pole position)}, 'ikw' {chain: 0..1}."""
    reset_pose(arm)
    pb = arm.pose.bones
    for name, r in pose.get("rot", {}).items():
        pb[name].rotation_quaternion = Euler([math.radians(x) for x in r], "XYZ").to_quaternion()
    dx, dy, dz = pose.get("hips", (0, 0, 0))
    pb["hips"].location = (dx, dz, -dy)   # hips local axes: X = world X, Y = up, Z = forward
    for key, *_ in ik_chain_specs():
        arm[f"ik_{key}"] = float(pose.get("ikw", {}).get(key, 0.0))
    for key, low, end, tgt, pole in ik_chain_specs():
        if key not in pose.get("ik", {}):
            continue
        pos, rot, pole_pos = pose["ik"][key]
        m = Matrix.Translation(pos) @ rot.to_4x4()
        pb[tgt].matrix_basis = pb[tgt].bone.matrix_local.inverted() @ m
        if pole_pos is not None:
            pm = Matrix.Translation(pole_pos)
            pb[pole].matrix_basis = pb[pole].bone.matrix_local.inverted() @ pm
    arm.update_tag()
    bpy.context.view_layer.update()


def read_basis(arm):
    """Local (parent-relative) transforms of the deform bones after constraints."""
    out = {}
    pb = arm.pose.bones
    for b in deform_bones(arm):
        p = pb[b.name]
        if b.parent:
            rest_rel = b.parent.matrix_local.inverted() @ b.matrix_local
            pose_rel = pb[b.parent.name].matrix.inverted() @ p.matrix
            basis = rest_rel.inverted() @ pose_rel
        else:
            basis = b.matrix_local.inverted() @ p.matrix
        loc, q, _s = basis.decompose()
        out[b.name] = (loc, q)
    return out


def write_action(arm, name, frames, loop):
    """frames: list of read_basis() results, one per frame from 0."""
    from bpy_extras import anim_utils
    act = bpy.data.actions.get(name)
    if act:
        bpy.data.actions.remove(act)
    act = bpy.data.actions.new(name)
    act.use_fake_user = True
    slot = act.slots.new(id_type="OBJECT", name=arm.name)
    cb = anim_utils.action_ensure_channelbag_for_slot(act, slot)
    n = len(frames)
    for b in deform_bones(arm):
        qs, prev = [], None
        for fr in frames:
            q = fr[b.name][1].copy()
            if prev is not None and q.dot(prev) < 0:
                q.negate()
            qs.append(q)
            prev = q
        path = f'pose.bones["{b.name}"]'
        chans = [("rotation_quaternion", i, [q[i] for q in qs]) for i in range(4)]
        if b.name in ("hips", "root"):
            chans += [("location", i, [fr[b.name][0][i] for fr in frames]) for i in range(3)]
        for prop, idx, vals in chans:
            fc = cb.fcurves.new(f"{path}.{prop}", index=idx, group_name=b.name)
            fc.keyframe_points.add(n)
            co = []
            for f, v in enumerate(vals):
                co += [float(f), float(v)]
            fc.keyframe_points.foreach_set("co", co)
            for kp in fc.keyframe_points:
                kp.interpolation = "LINEAR"
            fc.update()
    act.use_frame_range = True
    act.frame_start, act.frame_end = 0, n - 1
    act.use_cyclic = bool(loop)
    act["loop"] = bool(loop)
    return act, slot


def clear_toes(arm, basis, side, points):
    """A foot rolling onto its toes bends them up at the joint; it never pushes the toe tip into
    the floor. If the tip is below z = 0, turn the toes about their own hinge (X) just enough
    to bring it to the floor, in the baked frame `basis`. Returns the corrected sole points."""
    heel, ball, tip = points
    if tip.z >= 0.0:
        return points
    tb = arm.pose.bones[side + "Toes"]
    joint = tb.head.copy()
    axis = tb.matrix.col[0].to_3d().normalized()
    rel = tip - joint

    def z(deg):
        return (joint + Matrix.Rotation(math.radians(deg), 3, axis) @ rel).z
    sign = 1.0 if z(5) > z(-5) else -1.0
    lo, hi = 0.0, 90.0
    for _ in range(40):
        mid = (lo + hi) / 2
        if z(sign * mid) < 0:
            lo = mid
        else:
            hi = mid
    loc, q = basis[side + "Toes"]
    basis[side + "Toes"] = (loc, q @ Quaternion((1, 0, 0), math.radians(sign * hi)))
    return heel, ball, joint + Matrix.Rotation(math.radians(sign * hi), 3, axis) @ rel


def bake_clip(arm, clip):
    import anim_clips as AC
    frames, contacts = [], []
    for f in range(clip.length + 1):
        apply_pose(arm, clip.sample(f))
        basis = read_basis(arm)
        cp = {s: AC.contact_points(arm.pose.bones, s) for s in ("left", "right")}
        for s in ("left", "right"):
            cp[s] = clear_toes(arm, basis, s, cp[s])
        frames.append(basis)
        contacts.append(cp)
    act, slot = write_action(arm, clip.name, frames, clip.loop)
    if clip.speed:
        act["speed"] = clip.speed
    act["source"] = clip.meta.get("source", "procedural")
    return act, slot, frames, contacts


def assign_action(arm, act):
    ad = arm.animation_data or arm.animation_data_create()
    ad.action = act
    if act is not None:
        ad.action_slot = act.slots[0]


# ======================================================================== LODs, checks, export

def triangle_count(ob):
    return sum(len(p.vertices) - 2 for p in ob.data.polygons)


def max_influences(ob, arm):
    names = {b.name for b in arm.data.bones}
    idx = {g.index for g in ob.vertex_groups if g.name in names}
    return max(sum(1 for g in v.groups if g.group in idx and g.weight > 0) for v in ob.data.vertices)


def foot_slip(clip, contacts, speed):
    """Planted-foot slip: for consecutive planted frames, how far a ground contact point (heel or
    ball, whichever is on the ground in both frames) moves in world space once the controller's
    motion (speed along -Y, the character's forward) is added. Returns metres per frame per side."""
    res = []
    for side in ("left", "right"):
        worst = 0.0
        for f in range(clip.length):
            if not (clip.planted(side, f) and clip.planted(side, f + 1)):
                continue
            for k in range(2):
                p0, p1 = contacts[f][side][k], contacts[f + 1][side][k]
                if p0.z > 0.006 or p1.z > 0.006:
                    continue
                d0 = p0 + Vector((0, -speed * f / FPS, 0))
                d1 = p1 + Vector((0, -speed * (f + 1) / FPS, 0))
                worst = max(worst, Vector((d1.x - d0.x, d1.y - d0.y)).length)
        res.append(worst)
    return res


def ground_slip(clip, contacts):
    """For clips played standing still: the most a heel or ball point that touches the ground
    (within 6 mm) in two consecutive frames moves along the ground, per side, in metres per frame.
    A planted foot should not move at all."""
    res = []
    for side in ("left", "right"):
        worst = 0.0
        for f in range(clip.length):
            for k in range(2):
                p0, p1 = contacts[f][side][k], contacts[f + 1][side][k]
                if p0.z > 0.006 or p1.z > 0.006:
                    continue
                worst = max(worst, Vector((p1.x - p0.x, p1.y - p0.y)).length)
        res.append(worst)
    return res


# Clips played in the air, in water or on a ladder: their feet are not on the floor, so the floor
# check skips them (as it skips one-frame stills: concept poses and rig tests).
AIRBORNE = {"fall", "swim", "swim_idle", "climb"}
# Deepest a heel, ball or toe tip may go below the floor (z = 0) in any other clip.
FLOOR_TOLERANCE = 0.005


def floor_penetration(contacts):
    """Lowest heel, ball or toe-tip point over the clip (metres; negative = below the floor) and
    the frame and point where it happens."""
    worst = (float("inf"), None, None)
    for f, c in enumerate(contacts):
        for side in ("left", "right"):
            for name, p in zip(("heel", "ball", "toe"), c[side]):
                if p.z < worst[0]:
                    worst = (p.z, f, f"{side} {name}")
    return worst


def export_glb(path, arm, meshes, clips):
    # Key times in the glb are frame / scene fps: without this the factory 24 fps stretches every
    # clip by 30/24 (the first Phase B export had that bug).
    bpy.context.scene.render.fps = FPS
    bpy.context.scene.render.fps_base = 1.0
    bpy.ops.object.select_all(action="DESELECT")
    arm.select_set(True)
    hidden = [m for m in meshes if m.hide_get()]
    for m in meshes:
        m.hide_set(False)
        m.select_set(True)
    bpy.context.view_layer.objects.active = arm
    # One NLA track per exported clip; NLA_TRACKS mode writes exactly these (not the test poses
    # that also live in the .blend). The IK is off (FK only).
    for key, *_ in ik_chain_specs():
        arm[f"ik_{key}"] = 0.0
    reset_pose(arm)
    ad = arm.animation_data or arm.animation_data_create()
    ad.action = None
    for tr in list(ad.nla_tracks):
        ad.nla_tracks.remove(tr)
    for c in clips:
        act = bpy.data.actions[c.name]
        tr = ad.nla_tracks.new()
        tr.name = c.name
        st = tr.strips.new(c.name, 0, act)
        st.action_slot = act.slots[0]
    bpy.ops.export_scene.gltf(
        filepath=path, export_format="GLB", use_selection=True, export_apply=False,
        export_normals=True, export_materials="EXPORT", export_yup=True,
        export_skins=True, export_influence_nb=4, export_all_influences=False, export_def_bones=True,
        export_animations=True, export_animation_mode="NLA_TRACKS", export_force_sampling=True,
        export_frame_step=1, export_optimize_animation_size=True, export_anim_single_armature=True,
        export_extras=False, export_morph=False)
    for tr in list(ad.nla_tracks):
        ad.nla_tracks.remove(tr)
    for m in hidden:
        m.hide_set(True)
    return os.path.getsize(path)


def validate_glb(path, clips):
    """Re-import into an empty scene and check bones, influences, materials and clips."""
    bpy.ops.wm.read_factory_settings(use_empty=True)
    # The importer turns key times into frames at the scene rate; at the factory 24 fps a clip
    # exported at 24 fps would also read back with the right frame count, hiding a time-scale bug.
    bpy.context.scene.render.fps = FPS
    bpy.context.scene.render.fps_base = 1.0
    bpy.ops.import_scene.gltf(filepath=path)
    arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
    # The importer adds an "Icosphere" bone-display mesh of its own; only check the avatar meshes.
    meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.name.startswith("DefaultAvatar")]
    report = {"armatures": len(arms), "bones": len(arms[0].data.bones) if arms else 0, "meshes": {}}
    bone_names = {b.name for b in arms[0].data.bones}
    for m in sorted(meshes, key=lambda o: o.name):
        idx = {g.index for g in m.vertex_groups if g.name in bone_names}
        infl = max(sum(1 for g in v.groups if g.group in idx and g.weight > 1e-6) for v in m.data.vertices)
        unweighted = sum(1 for v in m.data.vertices if not any(g.group in idx and g.weight > 1e-6 for g in v.groups))
        report["meshes"][m.name] = {"triangles": triangle_count(m), "vertices": len(m.data.vertices),
                                    "max_influences": infl, "unweighted": unweighted,
                                    "materials": [s.material.name for s in m.material_slots if s.material]}
    want = {c.name: c.length for c in clips}
    got = {}
    for act in bpy.data.actions:
        nm = act.name.split("_DefaultAvatarRig")[0]
        fr = act.frame_range
        got[nm] = (round(fr[0]), round(fr[1]))
    report["clips"] = got
    missing = [n for n in want if n not in got]
    wrong = [n for n in want if n in got and got[n][1] - got[n][0] != want[n] and want[n] > 0]
    report["missing"], report["wrong_length"] = missing, wrong
    return report


# ======================================================================== main

def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-export", action="store_true")
    ap.add_argument("--only", nargs="*", default=None)
    ap.add_argument("--head-seg", type=int, default=None)
    a = ap.parse_args(argv)

    import anim_clips as AC

    lod0, t0 = BA.build(None, None, lod=0, head_seg=a.head_seg, reset=True, name="DefaultAvatar_LOD0")
    lod1, t1 = BA.build(None, None, lod=1, reset=False, name="DefaultAvatar_LOD1")
    lod2, t2 = BA.build(None, None, lod=2, reset=False, name="DefaultAvatar_LOD2")
    BA.LOD = 0
    arm = build_armature()
    stats = {"triangles": {"LOD0": t0, "LOD1": t1, "LOD2": t2}}
    for m in (lod0, lod1, lod2):
        stats.setdefault("max_influences", {})[m.name] = skin(m, arm)
    print(f"[rig] triangles {stats['triangles']}  influences {stats['max_influences']}")
    print(f"[rig] deform bones {len(deform_bones(arm))}: {[b.name for b in deform_bones(arm)]}")
    add_ik(arm)
    lod1.hide_render = lod2.hide_render = True
    lod1.hide_set(True)
    lod2.hide_set(True)

    clips = AC.all_clips(arm)
    if a.only:
        clips = [c for c in clips if c.name in a.only]
    info = {}
    below = []
    for c in clips:
        act, slot, frames, contacts = bake_clip(arm, c)
        entry = {"frames": c.length + 1, "first": 0, "last": c.length, "seconds": round(c.length / FPS, 4),
                 "loop": c.loop, "export": c.export, "priority": c.priority}
        entry.update(c.meta)
        if c.speed:
            slip = foot_slip(c, contacts, c.speed)
            entry["max_planted_foot_slip_mm_per_frame"] = [round(x * 1000, 2) for x in slip]
        elif c.length > 0:
            entry["max_ground_slip_mm_per_frame"] = [round(x * 1000, 2) for x in ground_slip(c, contacts)]
        z, f, where = floor_penetration(contacts)
        entry["min_sole_z_mm"] = round(z * 1000, 1)
        if c.name not in AIRBORNE and c.length > 0 and z < -FLOOR_TOLERANCE:
            below.append(f"{c.name}: {where} {z * 1000:.1f} mm below the floor at frame {f}")
        info[c.name] = entry
        print(f"[clip] {c.name}: 0..{c.length} loop={c.loop} {c.meta} "
              f"slip={entry.get('max_planted_foot_slip_mm_per_frame') or entry.get('max_ground_slip_mm_per_frame')}")
    if below:
        # Automatic check: no heel, ball or toe may sink into the floor in a grounded clip.
        raise SystemExit("[floor] FAIL: " + "; ".join(below))
    print(f"[floor] ok: no sole point more than {FLOOR_TOLERANCE * 1000:.0f} mm below the floor "
          f"(airborne clips {sorted(AIRBORNE)} skipped)")
    assign_action(arm, None)
    reset_pose(arm)
    # FK mode by default: the baked clips play as they are; an animator switches a limb to IK.
    for key, *_ in ik_chain_specs():
        arm[f"ik_{key}"] = 0.0
    bpy.context.view_layer.update()

    os.makedirs(OUT, exist_ok=True)
    blend = os.path.join(OUT, "default_avatar.blend")
    bpy.ops.wm.save_as_mainfile(filepath=blend, compress=True)
    print(f"[rig] saved {blend}")

    if not a.no_export and not a.only:
        exp = [c for c in clips if c.export]
        glb = os.path.join(OUT, "default_avatar.glb")
        size = export_glb(glb, arm, [lod0, lod1, lod2], exp)
        print(f"[export] {glb} bytes={size}")
        meta = {"file": "default_avatar.glb", "fps": FPS, "stud_m": BA.STUD,
                "meshes": {"DefaultAvatar_LOD0": t0, "DefaultAvatar_LOD1": t1, "DefaultAvatar_LOD2": t2},
                "clips": {c.name: info[c.name] for c in exp}}
        with open(os.path.join(OUT, "default_avatar_clips.json"), "w") as f:
            json.dump(meta, f, indent=1)
        rep = validate_glb(glb, exp)
        rep["bytes"] = size
        with open(os.path.join(OUT, "validation.json"), "w") as f:
            json.dump(rep, f, indent=1)
        print("[validate]", json.dumps(rep))


if __name__ == "__main__":
    main()
