"""Retarget the Quaternius Universal Animation Library (CC0) onto the default avatar's VRM rig.

Two steps:

1. Extract (inside Blender, once per download of the packs):
       blender -b --factory-startup --python art/avatar/retarget_clips.py -- extract --ual DIR
   DIR holds the unzipped Standard packs (see ANIMATION_SOURCES.md for where to get them and their
   checksums). The script imports each pack's glTF file into an empty scene, samples every clip
   it needs at 30 fps and writes DIR/cache/ual_sampled.json: per clip, per frame and per bone the
   armature-space rotation and joint position, with the source names already mapped to VRM names.
   Nothing from the packs is run; only the glTF data is read. The raw packs never enter the repo.

2. Retarget (pure maths, imported by anim_clips.py when rig_avatar.py bakes the clips). The
   `Retargeter` maps a source pose onto our skeleton by bone name and by rest-pose-corrected
   rotation, so it keeps working when the joints move (another branch reshapes the body):
   - Each bone gets an anatomical frame built from joint positions, the same way on both rigs:
     the direction to its child joint, and a second axis (forward for the trunk, legs and arms,
     sideways for the feet, across the knuckles for the hands and fingers). With R = a bone's
     armature-space rotation and A its anatomical frame at rest, the target bone takes
         R_tgt(t) = R_src(t) R_src_rest^-1 A_src A_tgt^-1 R_tgt_rest
     i.e. the anatomical frames of both rigs point the same way in the world at every frame. The
     source's T-pose and our A-pose rest therefore need no hand-made offsets.
   - Translations scale by the leg-length ratio `k` (target hip-to-ankle over source): the hips
     and the ankles move by k times the source's motion from rest, so a foot on the ground stays
     on our ground and the hips sit at our leg height.
   - Feet: ankle IK targets (see `Pose`) with the retargeted foot rotation, a pole from the
     retargeted knee, a reach limit (the hips drop rather than the knee snapping straight), sole
     points kept above the ground, and optional locking of planted feet so nothing slides.

The source of every clip and what was changed is listed in ANIMATION_SOURCES.md.
"""
import json
import math
import os
import sys

from mathutils import Matrix, Quaternion, Vector

FPS = 30
CACHE_NAME = "ual_sampled.json"

# ======================================================================== bone maps (source -> VRM)

VRM_FINGERS = ("Index", "Middle", "Ring", "Little")


def _map_ual1():
    """Universal Animation Library 1 (Rigify DEF- bones)."""
    m = {"DEF-hips": "hips", "DEF-spine.001": "spine", "DEF-spine.002": "chest", "DEF-spine.003": "upperChest",
         "DEF-neck": "neck", "DEF-head": "head"}
    for s, side in (("L", "left"), ("R", "right")):
        m.update({f"DEF-shoulder.{s}": side + "Shoulder", f"DEF-upper_arm.{s}": side + "UpperArm",
                  f"DEF-forearm.{s}": side + "LowerArm", f"DEF-hand.{s}": side + "Hand",
                  f"DEF-thigh.{s}": side + "UpperLeg", f"DEF-shin.{s}": side + "LowerLeg",
                  f"DEF-foot.{s}": side + "Foot", f"DEF-toe.{s}": side + "Toes"})
        for src, f in (("index", "Index"), ("middle", "Middle"), ("ring", "Ring"), ("pinky", "Little")):
            for i, part in enumerate(("Proximal", "Intermediate", "Distal")):
                m[f"DEF-f_{src}.0{i + 1}.{s}"] = side + f + part
        for i, part in enumerate(("Metacarpal", "Proximal", "Distal")):
            m[f"DEF-thumb.0{i + 1}.{s}"] = side + "Thumb" + part
    return m


def _map_ual2():
    """Universal Animation Library 2 (Unreal mannequin names)."""
    m = {"pelvis": "hips", "spine_01": "spine", "spine_02": "chest", "spine_03": "upperChest", "neck_01": "neck",
         "Head": "head"}
    for s, side in (("l", "left"), ("r", "right")):
        m.update({f"clavicle_{s}": side + "Shoulder", f"upperarm_{s}": side + "UpperArm",
                  f"lowerarm_{s}": side + "LowerArm", f"hand_{s}": side + "Hand", f"thigh_{s}": side + "UpperLeg",
                  f"calf_{s}": side + "LowerLeg", f"foot_{s}": side + "Foot", f"ball_{s}": side + "Toes"})
        for src, f in (("index", "Index"), ("middle", "Middle"), ("ring", "Ring"), ("pinky", "Little")):
            for i, part in enumerate(("Proximal", "Intermediate", "Distal")):
                m[f"{src}_0{i + 1}_{s}"] = side + f + part
        for i, part in enumerate(("Metacarpal", "Proximal", "Distal")):
            m[f"thumb_0{i + 1}_{s}"] = side + "Thumb" + part
    return m


# pack key -> (glb file name, bone map, clips to sample)
PACKS = {
    "ual1": ("AnimationLibrary_Godot_Standard.glb", _map_ual1(),
             ["Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop", "Sprint_Loop", "Jump_Start", "Jump_Loop", "Jump_Land",
              "Swim_Fwd_Loop", "Swim_Idle_Loop", "Sitting_Idle_Loop", "Dance_Loop"]),
    "ual2": ("UAL2_Standard.glb", _map_ual2(),
             ["Idle_FoldArms_Loop"]),
}


# ======================================================================== step 1: extract (Blender)

def _find(root, name):
    for d, _dirs, files in os.walk(root):
        if name in files:
            return os.path.join(d, name)
    return None


def extract(ual_dir):
    import bpy
    out = {"fps": FPS, "packs": {}}
    for key, (glb, bmap, wanted) in PACKS.items():
        path = _find(ual_dir, glb)
        if path is None:
            raise SystemExit(f"[retarget] {glb} not found under {ual_dir}")
        bpy.ops.wm.read_factory_settings(use_empty=True)
        sc = bpy.context.scene
        sc.render.fps, sc.render.fps_base = FPS, 1.0
        bpy.ops.import_scene.gltf(filepath=path)
        arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
        mw = arm.matrix_world.copy()
        names = [b.name for b in arm.data.bones if b.name in bmap]
        missing = sorted(set(bmap) - set(names))
        if missing:
            raise SystemExit(f"[retarget] {key}: source bones missing {missing}")
        rest = {}
        for n in names:
            b = arm.data.bones[n]
            m = mw @ b.matrix_local
            rest[bmap[n]] = {"q": list(m.to_quaternion()), "head": list(m.translation)}
        clips = {}
        ad = arm.animation_data or arm.animation_data_create()
        for cname in wanted:
            act = bpy.data.actions.get(cname)
            if act is None:
                raise SystemExit(f"[retarget] {key}: clip {cname} missing")
            ad.action = act
            if act.slots:
                ad.action_slot = act.slots[0]
            f0, f1 = (int(round(x)) for x in act.frame_range)
            frames = []
            for f in range(f0, f1 + 1):
                sc.frame_set(f)
                fr = {}
                for n in names:
                    m = mw @ arm.pose.bones[n].matrix
                    q = m.to_quaternion()
                    fr[bmap[n]] = [round(x, 6) for x in list(q) + list(m.translation)]
                frames.append(fr)
            clips[cname] = {"frames": frames, "loop": cname.endswith("_Loop")}
            print(f"[retarget] {key}/{cname}: {len(frames)} frames")
        out["packs"][key] = {"file": glb, "rest": rest, "clips": clips}
    cache = os.path.join(ual_dir, "cache")
    os.makedirs(cache, exist_ok=True)
    path = os.path.join(cache, CACHE_NAME)
    with open(path, "w") as f:
        json.dump(out, f, separators=(",", ":"))
    print(f"[retarget] wrote {path}")


# ======================================================================== source data

def cache_path():
    """The sampled-source cache: $METALLIC_UAL/cache/ual_sampled.json."""
    d = os.environ.get("METALLIC_UAL")
    if not d:
        raise SystemExit("[retarget] set METALLIC_UAL to the folder with the unzipped UAL packs "
                         "(see art/avatar/ANIMATION_SOURCES.md) and run the extract step once")
    p = os.path.join(d, "cache", CACHE_NAME)
    if not os.path.exists(p):
        raise SystemExit(f"[retarget] {p} missing: run the extract step (see retarget_clips.py)")
    return p


class SourceClip:
    def __init__(self, name, data):
        self.name = name
        self.loop = data["loop"]
        self.frames = []
        for fr in data["frames"]:
            self.frames.append({b: (Quaternion(v[:4]), Vector(v[4:7])) for b, v in fr.items()})
        self.last = len(self.frames) - 1      # a loop's last frame repeats its first

    def at(self, f):
        """Bone -> (rotation, joint position) at fractional frame f (wrapped for loops, clamped
        otherwise), slerped between frames."""
        if self.loop and self.last > 0:
            f = f % self.last
        else:
            f = max(0.0, min(float(self.last), f))
        i = min(int(math.floor(f)), self.last)
        j = min(i + 1, self.last)
        u = f - i
        a, b = self.frames[i], self.frames[j]
        if u < 1e-6:
            return a
        return {n: (a[n][0].slerp(b[n][0], u), a[n][1].lerp(b[n][1], u)) for n in a}


class Source:
    def __init__(self, path=None):
        with open(path or cache_path()) as f:
            data = json.load(f)
        self.packs = {}
        for key, p in data["packs"].items():
            rest = {b: (Quaternion(v["q"]), Vector(v["head"])) for b, v in p["rest"].items()}
            clips = {n: SourceClip(n, c) for n, c in p["clips"].items()}
            self.packs[key] = (rest, clips)

    def clip(self, pack, name):
        return self.packs[pack][1][name]

    def rest(self, pack):
        return self.packs[pack][0]


# ======================================================================== anatomical frames

SIDES = ("left", "right")


def _child_table():
    """bone -> the joint its direction points at (another bone's head), or a fixed world vector."""
    t = {"hips": "spine", "spine": "chest", "chest": "upperChest", "upperChest": "neck", "neck": "head",
         "head": Vector((0, 0, 1))}
    for s in SIDES:
        t.update({s + "Shoulder": s + "UpperArm", s + "UpperArm": s + "LowerArm", s + "LowerArm": s + "Hand",
                  s + "Hand": s + "MiddleProximal", s + "UpperLeg": s + "LowerLeg", s + "LowerLeg": s + "Foot",
                  s + "Foot": s + "Toes", s + "Toes": Vector((0, -1, 0))})
        for f in VRM_FINGERS:
            t[s + f + "Proximal"] = s + f + "Intermediate"
            t[s + f + "Intermediate"] = s + f + "Distal"
            t[s + f + "Distal"] = ("same", s + f + "Intermediate")
        t[s + "ThumbMetacarpal"] = s + "ThumbProximal"
        t[s + "ThumbProximal"] = s + "ThumbDistal"
        t[s + "ThumbDistal"] = ("same", s + "ThumbProximal")
    return t


CHILD = _child_table()
FORWARD = Vector((0, -1, 0))
LATERAL = Vector((1, 0, 0))


def _frame(primary, secondary):
    p = primary.normalized()
    s = secondary - p * secondary.dot(p)
    if s.length < 1e-6:
        s = Vector((0, 0, 1)) - p * p.z
    s.normalize()
    return Matrix((p, s, p.cross(s))).transposed()


def anatomical_frames(heads):
    """bone -> 3x3 frame at rest from joint positions `heads` (bone -> Vector)."""
    def direction(b):
        c = CHILD[b]
        if isinstance(c, Vector):
            return c.copy()
        if isinstance(c, tuple):
            return direction(c[1])
        return heads[c] - heads[b]

    out = {}
    for b in heads:
        if b not in CHILD:
            continue
        side = "left" if b.startswith("left") else ("right" if b.startswith("right") else None)
        d = direction(b)
        if side and any(k in b for k in ("Hand", "Thumb", "Index", "Middle", "Ring", "Little")):
            across = heads[side + "IndexProximal"] - heads[side + "LittleProximal"]
            if "Thumb" in b:
                hand_dir = heads[side + "MiddleProximal"] - heads[side + "Hand"]
                sec = hand_dir.cross(across)
            else:
                sec = across
        elif b.endswith(("Foot", "Toes")):
            sec = LATERAL
        else:
            sec = FORWARD
        out[b] = _frame(d, sec)
    return out


# ======================================================================== retargeting

class Retargeter:
    """Maps one source pack onto the target rig (an anim_clips.Rig: rest matrices in `rig.rest`,
    parents from the armature)."""

    def __init__(self, rig, source, pack):
        self.rig = rig
        self.pack = pack
        self.src_rest = source.rest(pack)
        self.source = source
        bones = rig.arm.data.bones
        self.parent = {b.name: (b.parent.name if b.parent else None) for b in bones}
        self.order = [b.name for b in bones if b.use_deform and b.name in self.src_rest]
        self.tgt_rot = {n: rig.rest[n].to_3x3().normalized() for n in self.order}
        self.tgt_head = {n: rig.rest[n].translation.copy() for n in self.order}
        src_head = {n: v[1] for n, v in self.src_rest.items()}
        a_src = anatomical_frames(src_head)
        a_tgt = anatomical_frames(self.tgt_head)
        # R_tgt(t) = R_src(t) @ M[b]
        self.M = {}
        for n in self.order:
            r_src = self.src_rest[n][0].to_matrix()
            self.M[n] = r_src.inverted() @ a_src[n] @ a_tgt[n].inverted() @ self.tgt_rot[n]
        self.k = self.leg_length(self.tgt_head) / self.leg_length(src_head)

    @staticmethod
    def leg_length(h):
        return ((h["leftLowerLeg"] - h["leftUpperLeg"]).length + (h["leftFoot"] - h["leftLowerLeg"]).length)

    # ---------------------------------------------------------------- per frame
    def world(self, clip, f):
        """Target armature-space rotations (bone -> 3x3) and the hips joint position at source
        frame f."""
        sp = self.source.clip(self.pack, clip).at(f)
        rots = {n: sp[n][0].to_matrix() @ self.M[n] for n in self.order}
        hips = self.tgt_head["hips"] + (sp["hips"][1] - self.src_rest["hips"][1]) * self.k
        return rots, hips, sp

    def src_point(self, sp, bone):
        """A source joint, moved into target space (rest + k x motion from rest)."""
        return self.tgt_head[bone] + (sp[bone][1] - self.src_rest[bone][1]) * self.k

    def joint(self, rots, hips, bone):
        """FK position of a target joint from world rotations and the hips position."""
        chain = []
        b = bone
        while b and b != "hips":
            chain.append(b)
            b = self.parent[b]
        pos = hips.copy()
        prev = "hips"
        for b in reversed(chain):
            off = self.tgt_head[b] - self.tgt_head[prev]
            pos = pos + rots[prev] @ self.tgt_rot[prev].inverted() @ off
            prev = b
        return pos

    def local(self, rots):
        """World rotations -> local basis quaternions (what the pose bones take)."""
        out = {}
        for n in self.order:
            p = self.parent[n]
            rc_rest, rc = self.tgt_rot[n], rots[n]
            if p in rots:
                rp_rest, rp = self.tgt_rot[p], rots[p]
                basis = rc_rest.transposed() @ rp_rest @ rp.transposed() @ rc
            else:
                basis = rc_rest.transposed() @ rc
            out[n] = basis.to_quaternion().normalized()
        return out


def rotate_subtree(rots, parent, bone, W):
    """Rotate `bone` and everything under it by the world rotation W (pre-multiplied)."""
    kids = {bone}
    changed = True
    while changed:
        changed = False
        for n, p in parent.items():
            if p in kids and n not in kids and n in rots:
                kids.add(n)
                changed = True
    for n in kids:
        if n in rots:
            rots[n] = W @ rots[n]


def quat_to_euler_deg(q):
    e = q.to_euler("XYZ")
    return tuple(math.degrees(x) for x in e)


# ======================================================================== clips

def wrap_or_clamp(f, length, loop):
    if loop and length > 0:
        return f % length
    return max(0.0, min(float(length), f))


def qpow(q, s):
    """q raised to the power s (scales its rotation angle)."""
    axis, ang = q.to_axis_angle()
    if abs(ang) < 1e-9:
        return Quaternion()
    if ang > math.pi:
        ang -= 2 * math.pi
    return Quaternion(axis, ang * s)


def mean_quat(qs):
    """Average of nearby unit quaternions (sign-aligned sum, normalised)."""
    ref = qs[0]
    acc = [0.0, 0.0, 0.0, 0.0]
    for q in qs:
        q = q if q.dot(ref) >= 0 else -q
        acc = [a + b for a, b in zip(acc, q)]
    return Quaternion(acc).normalized()


def euler_q(r):
    from mathutils import Euler
    return Euler([math.radians(x) for x in r], "XYZ").to_quaternion()


class Ctx:
    """What a polish layer sees and edits: target frame f (0..length), the clip, world rotations
    (bone -> 3x3), the hips joint position and the raw source pose."""

    def __init__(self, clip, f, rots, hips, sp):
        self.clip, self.f, self.rots, self.hips, self.sp = clip, f, rots, hips, sp
        self.u = f / clip.length if clip.length else 0.0     # 0..1 through the clip

    def rotate(self, bone, W):
        rotate_subtree(self.rots, self.clip.R.parent, bone, W)

    def axis(self, bone, i):
        """World direction of a bone's own rest axis i (0 = X hinge, 1 = Y along, 2 = Z side)."""
        return (self.rots[bone] @ self.clip.R.tgt_rot[bone].inverted() @ self.clip.R.tgt_rot[bone].col[i]).normalized()

    def rotate_local(self, bone, i, deg):
        """Rotate a bone (and its children) about its own axis i by `deg` degrees."""
        self.rotate(bone, Matrix.Rotation(math.radians(deg), 3, self.axis(bone, i)))

    def rotate_world(self, bone, axis, deg):
        self.rotate(bone, Matrix.Rotation(math.radians(deg), 3, Vector(axis)))


# ---------------------------------------------------------------- polish layers (ctx -> None)

def level_head(k=0.7, neck_share=0.35, pitch_bias=0.0):
    """Turn the head toward upright by k of its tilt (pitch and roll; yaw is kept), part of it
    in the neck. The big head reads as a bob or a slump when it follows the trunk; a level head
    looks where the character goes. pitch_bias > 0 tips the face down by that many degrees."""
    def layer(ctx):
        up = ctx.axis("head", 1)
        target = Matrix.Rotation(math.radians(pitch_bias), 3, ctx.axis("hips", 0)) @ Vector((0, 0, 1))
        q = up.rotation_difference(target)
        ctx.rotate("neck", qpow(q, k * neck_share).to_matrix())
        up = ctx.axis("head", 1)
        q = up.rotation_difference(target)
        ctx.rotate("head", qpow(q, k * (1 - neck_share) / max(1e-3, 1 - k * neck_share)).to_matrix())
    return layer


def abduct(deg, bones=("UpperArm",)):
    """Swing the arms out from the body by `deg` (a number or a function of ctx), so the chunky
    arms clear the wide torso."""
    def layer(ctx):
        d = deg(ctx) if callable(deg) else deg
        for side, ax in (("left", Vector((0, -1, 0))), ("right", Vector((0, 1, 0)))):
            for b in bones:
                ctx.rotate(side + b, Matrix.Rotation(math.radians(d), 3, ax))
    return layer


def turn(bone, axis, deg):
    """Rotate a bone's subtree about a world axis by `deg` (number or function of ctx)."""
    def layer(ctx):
        d = deg(ctx) if callable(deg) else deg
        if d:
            ctx.rotate_world(bone, axis, d)
    return layer


def bend(bone, i, deg):
    """Rotate a bone's subtree about its own rest axis i (0 hinge X, 1 twist Y, 2 side Z)."""
    def layer(ctx):
        d = deg(ctx) if callable(deg) else deg
        if d:
            ctx.rotate_local(bone, i, d)
    return layer


def toe_out(deg):
    """Turn both feet out by `deg` (the concept's stance). The feet are IK targets, so this turns
    the planted foot on the ground, not the whole leg."""
    def layer(ctx):
        ctx.rotate_world("leftFoot", (0, 0, 1), deg)
        ctx.rotate_world("rightFoot", (0, 0, 1), -deg)
    return layer


def piecewise(points):
    """Target frame -> source frame through (target, source) points, linear between them."""
    pts = sorted(points)

    def m(f):
        if f <= pts[0][0]:
            return pts[0][1]
        for (a, sa), (b, sb) in zip(pts, pts[1:]):
            if f <= b:
                return sa + (sb - sa) * (f - a) / (b - a)
        return pts[-1][1]
    return m


def ease(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3 - 2 * t)


def keyed(keys, f, mix):
    """Value at frame f of [(frame, value), ...]: held before the first key and after the last,
    eased (smoothstep) between keys, blended with mix(a, b, w)."""
    if f <= keys[0][0]:
        return keys[0][1]
    for (fa, va), (fb, vb) in zip(keys, keys[1:]):
        if f <= fb:
            return mix(va, vb, ease((f - fa) / (fb - fa)))
    return keys[-1][1]


def _mix_dir(a, b, w):
    a, b = Vector(a).normalized(), Vector(b).normalized()
    return a.slerp(b, w) if a.dot(b) > -0.999 else (a * (1 - w) + b * w).normalized()


def _mix_num(a, b, w):
    return a + (b - a) * w


def aim(bone, keys, weight=None, mirror=True):
    """Swing a bone (and its subtree) so that it points along a world direction, keyed over time.

    keys: [(frame, (x, y, z)), ...] in the target rig's space, as written for `bone`; with mirror
    and a "left..." bone, the right side gets the same keys with x negated. Blender
    world: -Y is forward, +Z up, +X the character's left. weight: [(frame, w), ...] (default 1),
    blending from the clip's own pose to the aimed one. Directions are slerped between keys, so
    a limb moves on an arc. The swing is the shortest one, so the bone keeps its twist."""
    sides = [(bone, 1)]
    if mirror and bone.startswith("left"):
        sides.append(("right" + bone[4:], -1))

    def layer(ctx):
        w = keyed(weight, ctx.f, _mix_num) if weight else 1.0
        if w <= 0:
            return
        for b, sx in sides:
            d = Vector(keyed(keys, ctx.f, _mix_dir))
            d.x *= sx
            cur = ctx.axis(b, 1)
            q = cur.rotation_difference(d.normalized())
            ctx.rotate(b, qpow(q, w).to_matrix())
    return layer


def spin(bone, axis_index, keys, mirror=True, sign_right=-1):
    """Rotate a bone about its own axis (0 = X hinge, 1 = Y along, 2 = Z side) by keyed degrees
    [(frame, deg), ...]; the right side mirrored with sign_right."""
    sides = [(bone, 1)]
    if mirror and bone.startswith("left"):
        sides.append(("right" + bone[4:], sign_right))

    def layer(ctx):
        d = keyed(keys, ctx.f, _mix_num)
        for b, sg in sides:
            if d:
                ctx.rotate_local(b, axis_index, d * sg)
    return layer


def lean(bone, keys):
    """Rotate a bone's subtree about world axes by keyed (pitch, roll, yaw) degrees: positive
    pitch tips the top forward (about +X), positive roll tips it toward the character's left
    (+X, about +Y), positive yaw turns it toward the left (about +Z)."""
    def layer(ctx):
        p, r, y = keyed(keys, ctx.f, lambda a, b, w: tuple(x + (z - x) * w for x, z in zip(a, b)))
        if p:
            ctx.rotate_world(bone, (1, 0, 0), p)
        if r:
            ctx.rotate_world(bone, (0, 1, 0), r)
        if y:
            ctx.rotate_world(bone, (0, 0, 1), y)
    return layer


def hips_keys(keys):
    """Keyed world offset of the hips [(frame, (dx, dy, dz)), ...]."""
    def layer(ctx):
        d = keyed(keys, ctx.f, lambda a, b, w: tuple(x + (z - x) * w for x, z in zip(a, b)))
        ctx.hips = ctx.hips + Vector(d)
    return layer


def move_hips(fn):
    """Add a world offset (function of ctx -> Vector) to the hips."""
    def layer(ctx):
        ctx.hips = ctx.hips + Vector(fn(ctx))
    return layer


class RetargetClip:
    """A clip made from a source clip: retargeted, then polished in code.

    src           source clip name in the retargeter's pack
    length        target frames (last frame = length; loops repeat frame 0 there)
    timemap       target frame -> source frame (default: the source stretched to `length`)
    feet          "lock": ankle IK with planted feet locked; "ik": ankle IK; "fk": rotations only
    layers        functions(ctx) run on the world pose each frame (arcs, breathing, lean, ...)
    lag           {bone or pattern: frames} overlap: the bone's local rotation is sampled late
    hands         {side: (shape, weight)} or callable(f) -> that; blends the fingers toward a
                  hand shape from anim_clips.hand()
    stabilize     {bone: k}: k of the bone's swing away from its clip-mean world rotation is
                  removed (keeps the big head steady)
    exaggerate    {bone or pattern: s}: local rotation away from the clip mean scaled by s
    """

    def __init__(self, rig, R, name, src, length, loop, priority=1, export=True, timemap=None, feet="lock",
                 layers=(), lag=None, hands=None, stabilize=None, exaggerate=None, meta=None, ground_lock=0.025,
                 lock_speed=0.25, speed=None, spread=None):
        self.rig, self.R, self.name, self.src, self.length, self.loop = rig, R, name, src, length, loop
        self.priority, self.export, self.meta, self.speed = priority, export, dict(meta or {}), speed
        sclip = R.source.clip(R.pack, src)
        if timemap is None:
            sc = sclip.last / length if length else 1.0
            timemap = (lambda f, sc=sc: f * sc)
        self.timemap = timemap
        self.feet, self.layers, self.lag = feet, list(layers), dict(lag or {})
        self.hands, self.stab, self.exag = hands, dict(stabilize or {}), dict(exaggerate or {})
        self.ground_lock, self.lock_speed, self.spread = ground_lock, lock_speed, spread
        self._base = {}
        self._mean = None
        self._locks = None
        self._xm = None
        self.meta.setdefault("source", f"Quaternius {('UAL 1' if R.pack == 'ual1' else 'UAL 2')} {src}")

    def planted(self, side, f):
        return False

    # ---------------------------------------------------------------- helpers
    @staticmethod
    def _match(table, bone):
        for k, v in table.items():
            if k == bone or (k.endswith("*") and bone.startswith(k[:-1])) or \
                    (k.startswith("*") and bone.endswith(k[1:])) or \
                    ("*" in k[1:-1] and bone.startswith(k.split("*")[0]) and bone.endswith(k.split("*")[1])):
                return v
        return None

    def _world(self, f):
        """World pose after retargeting, head steadying and the polish layers (no lag)."""
        f = wrap_or_clamp(f, self.length, self.loop)
        rots, hips, sp = self.R.world(self.src, self.timemap(f))
        if self.stab:
            means = self._means()
            for b, k in self.stab.items():
                cur = rots[b].to_quaternion()
                m = means[b] if means[b].dot(cur) >= 0 else -means[b]
                W = (cur.slerp(m, k) @ cur.inverted()).to_matrix()
                rotate_subtree(rots, self.R.parent, b, W)
        ctx = Ctx(self, f, rots, hips, sp)
        for layer in self.layers:
            layer(ctx)
        return ctx

    def _means(self):
        if self._mean is None:
            n = max(1, int(self.length))
            acc = {b: [] for b in self.stab}
            for i in range(n + (0 if self.loop else 1)):
                rots, _h, _s = self.R.world(self.src, self.timemap(i))
                for b in acc:
                    acc[b].append(rots[b].to_quaternion())
            self._mean = {b: mean_quat(qs) for b, qs in acc.items()}
        return self._mean

    def _local(self, f):
        key = round(wrap_or_clamp(f, self.length, self.loop), 4)
        if key not in self._base:
            ctx = self._world(key)
            self._base[key] = (self.R.local(ctx.rots), ctx)
        return self._base[key]

    def _exag_means(self):
        if self._xm is None:
            n = max(1, int(self.length))
            acc = {}
            for i in range(n + (0 if self.loop else 1)):
                loc, _ = self._local(i)
                for b, q in loc.items():
                    if self._match(self.exag, b) is not None:
                        acc.setdefault(b, []).append(q)
            self._xm = {b: mean_quat(qs) for b, qs in acc.items()}
        return self._xm

    # ---------------------------------------------------------------- feet
    def _raw_ankles(self, f):
        _loc, ctx = self._local(f)
        out = {}
        for s in SIDES:
            a = self.R.src_point(ctx.sp, s + "Foot")
            if self.spread:
                # Pull the feet toward (or push them from) where they stand in the rest pose.
                p = self.R.tgt_head[s + "Foot"]
                a = Vector((p.x + (a.x - p.x) * self.spread[0], p.y + (a.y - p.y) * self.spread[1], a.z))
            out[s] = a
        return out

    def _lock_table(self):
        """Per side and frame: (weight, locked position). A foot is planted while its ankle is
        within `ground_lock` of its rest height and moves slower than `lock_speed` m/s; it is held
        at the segment's mean position, eased in and out over 2 frames (not at a loop seam or the
        clip's ends, where the segment continues)."""
        if self._locks is not None:
            return self._locks
        n = int(self.length)
        raw = [self._raw_ankles(f) for f in range(n + 1)]
        out = {}
        for s in SIDES:
            rest_z = self.R.tgt_head[s + "Foot"].z
            pl = []
            for i in range(n + 1):
                a = raw[i][s]
                b = raw[i + 1][s] if i < n else (raw[1][s] if self.loop and n > 1 else raw[i - 1][s])
                v = Vector((a.x - b.x, a.y - b.y)).length * FPS
                pl.append(a.z - rest_z < self.ground_lock and v < self.lock_speed)
            if self.loop:
                pl[n] = pl[0]
            segs, i = [], 0
            while i <= n:
                if pl[i]:
                    j = i
                    while j + 1 <= n and pl[j + 1]:
                        j += 1
                    segs.append([i, j])
                    i = j + 1
                else:
                    i += 1
            # a loop's first and last segments are one segment across the seam
            if self.loop and len(segs) > 1 and segs[0][0] == 0 and segs[-1][1] == n:
                first, last = segs.pop(0), segs.pop()
                segs.append([last[0], first[1] + n])
            w = [0.0] * (n + 1)
            pos = [None] * (n + 1)
            for i0, i1 in segs:
                idx = [i % n if self.loop and n else i for i in range(i0, i1 + 1)]
                # Lock the middle of the sole, not the ankle: a foot that turns on the spot then
                # pivots about its sole instead of swinging its toes and heel across the ground.
                pts = [self.sole_centre(s, raw[i][s], i) for i in idx]
                m = Vector((sum(p.x for p in pts) / len(pts), sum(p.y for p in pts) / len(pts), 0.0))
                whole = self.loop and (i1 - i0) >= n
                for k, i in enumerate(idx):
                    e_in = 1.0 if (whole or (not self.loop and i0 == 0)) else min(1.0, (k + 1) / 2.0)
                    e_out = 1.0 if (whole or (not self.loop and i1 == n)) else min(1.0, (len(idx) - k) / 2.0)
                    w[i], pos[i] = min(e_in, e_out), m
            if self.loop:
                w[n], pos[n] = w[0], pos[0]
            out[s] = (w, pos)
        self._locks = out
        return out

    # ---------------------------------------------------------------- sample
    def sample(self, f):
        R = self.R
        loc, ctx = self._local(f)
        loc = dict(loc)
        if self.lag:
            for b in list(loc):
                lg = self._match(self.lag, b)
                if lg:
                    loc[b] = self._local(f - lg)[0][b]
        if self.exag:
            for b, m in self._exag_means().items():
                s = self._match(self.exag, b)
                loc[b] = m @ qpow(m.inverted() @ loc[b], s)
        hands = self.hands(f) if callable(self.hands) else self.hands
        if hands:
            import anim_clips as AC
            for side, (shape, w) in hands.items():
                for b, r in AC.hand(shape, side).items():
                    q = euler_q(r)
                    if loc[b].dot(q) < 0:
                        q = -q
                    loc[b] = loc[b].slerp(q, w)
        hips = ctx.hips.copy()
        pose = {"rot": {b: quat_to_euler_deg(q) for b, q in loc.items()}, "ik": {}, "ikw": {}}
        if self.feet != "fk":
            ankles = dict(self._final_ankles(int(round(wrap_or_clamp(f, self.length, self.loop)))))
            hips = self.place_legs(pose, ctx.rots, hips, ankles)
        pose["hips"] = tuple(hips - R.tgt_head["hips"])
        return pose

    def _final_ankles(self, fi):
        """Ankle targets at integer frame fi: source ankles, locked while planted, soles kept
        above the ground, and then any heel or ball that touches the ground in this frame and
        the one before held where it was, so a foot that rolls onto its toes or turns on the spot
        pivots about the contact instead of skating."""
        if not hasattr(self, "_fa"):
            self._fa = {}
        if fi in self._fa:
            return self._fa[fi]
        ankles = self._raw_ankles(fi)
        if self.feet == "lock":
            lt = self._lock_table()
            for s in SIDES:
                w, pos = lt[s][0][fi], lt[s][1][fi]
                if pos is not None and w > 0:
                    a = ankles[s]
                    c = self.sole_centre(s, a, fi)
                    ankles[s] = Vector((a.x + (pos.x - c.x) * w, a.y + (pos.y - c.y) * w, a.z))
        rots = self._local(fi)[1].rots
        for s in SIDES:
            low = min(p.z for p in self.sole_points(s, ankles[s], rots[s + "Foot"]))
            if low < 0:
                ankles[s] = ankles[s] + Vector((0, 0, -low))
        if fi > 0 and not self.loop:     # loops rely on the lock (a pin would drift across the seam)
            prev = self._final_ankles(fi - 1)
            prots = self._local(fi - 1)[1].rots
            for s in SIDES:
                now_pts = self.sole_points(s, ankles[s], rots[s + "Foot"])
                was_pts = self.sole_points(s, prev[s], prots[s + "Foot"])
                k = min(range(2), key=lambda i: now_pts[i].z)
                if now_pts[k].z < 0.012 and was_pts[k].z < 0.012:
                    d = was_pts[k] - now_pts[k]
                    ankles[s] = ankles[s] + Vector((d.x, d.y, 0.0))
        self._fa[fi] = ankles
        return ankles

    def place_legs(self, pose, rots, hips, ankles):
        """Leg IK targets: soles above the ground, the hips lowered when a leg would over-reach,
        the knee pole from the retargeted knee. Returns the (maybe lowered) hips position."""
        R = self.R
        foot_w = {s: rots[s + "Foot"] for s in SIDES}
        for s in SIDES:
            low = min(p.z for p in self.sole_points(s, ankles[s], foot_w[s]))
            if low < 0:
                ankles[s] = ankles[s] + Vector((0, 0, -low))
        L = R.leg_length(R.tgt_head)
        hip_rot = rots["hips"] @ R.tgt_rot["hips"].inverted()
        reach = 0.985 * L
        drop = 0.0
        for s in SIDES:
            hj = hips + hip_rot @ (R.tgt_head[s + "UpperLeg"] - R.tgt_head["hips"])
            d = ankles[s] - hj
            if d.length > reach:
                horiz2 = d.x * d.x + d.y * d.y
                drop = max(drop, (hj.z - ankles[s].z) - math.sqrt(max(0.0, reach * reach - horiz2)))
        hips = hips - Vector((0, 0, drop))
        for s in SIDES:
            hj = hips + hip_rot @ (R.tgt_head[s + "UpperLeg"] - R.tgt_head["hips"])
            knee = hj + rots[s + "UpperLeg"] @ R.tgt_rot[s + "UpperLeg"].inverted() @ \
                (R.tgt_head[s + "LowerLeg"] - R.tgt_head[s + "UpperLeg"])
            dirv = knee - (hj + ankles[s]) / 2
            if dirv.length < 0.02:
                dirv = hip_rot @ Vector((0, -1, 0))
            pose["ik"][f"leg.{s}"] = (ankles[s], foot_w[s], knee + dirv.normalized() * 0.5)
            pose["ikw"][f"leg.{s}"] = 1.0
        return hips

    def sole_centre(self, side, ankle, fi):
        h, b = self.sole_points(side, ankle, self._local(fi)[1].rots[side + "Foot"])
        return (h + b) / 2

    def sole_points(self, side, ankle, foot_world):
        """World heel and ball points for an ankle position and foot rotation."""
        import anim_clips as AC
        s = 1 if side == "left" else -1
        back = foot_world @ self.R.tgt_rot[side + "Foot"].inverted()
        return [ankle + back @ Vector((rel.x * s, rel.y, rel.z)) for rel in (AC.HEEL_REL, AC.BALL_REL)]


def blend_pose(a, b, w):
    """Blend two baked-format poses (rotations slerped, hips and IK targets interpolated)."""
    if w <= 0:
        return a
    if w >= 1:
        return b
    rot = {}
    for n in set(a["rot"]) | set(b["rot"]):
        qa = euler_q(a["rot"].get(n, (0, 0, 0)))
        qb = euler_q(b["rot"].get(n, (0, 0, 0)))
        if qa.dot(qb) < 0:
            qb = -qb
        rot[n] = quat_to_euler_deg(qa.slerp(qb, w))
    hips = tuple(x + (y - x) * w for x, y in zip(a.get("hips", (0, 0, 0)), b.get("hips", (0, 0, 0))))
    ik, ikw = {}, {}
    for ch in set(a.get("ik", {})) | set(b.get("ik", {})):
        if ch in a.get("ik", {}) and ch in b.get("ik", {}):
            (pa, ra, la), (pb, rb, lb) = a["ik"][ch], b["ik"][ch]
            qa, qb = ra.to_quaternion(), rb.to_quaternion()
            if qa.dot(qb) < 0:
                qb = -qb
            ik[ch] = (pa.lerp(pb, w), qa.slerp(qb, w).to_matrix(), la.lerp(lb, w))
            ikw[ch] = a["ikw"].get(ch, 1.0) + (b["ikw"].get(ch, 1.0) - a["ikw"].get(ch, 1.0)) * w
        else:
            src = a if ch in a.get("ik", {}) else b
            ik[ch] = src["ik"][ch]
            ikw[ch] = src["ikw"].get(ch, 1.0) * ((1 - w) if src is a else w)
    return {"rot": rot, "hips": hips, "ik": ik, "ikw": ikw}


class FnClip:
    """A clip whose pose is any function of the frame (used to chain and blend retargeted clips)."""

    def __init__(self, name, length, loop, fn, priority=1, export=True, meta=None, speed=None):
        self.name, self.length, self.loop, self.fn = name, length, loop, fn
        self.priority, self.export, self.meta, self.speed = priority, export, dict(meta or {}), speed

    def planted(self, side, f):
        return False

    def sample(self, f):
        return self.fn(f)


# ======================================================================== main

def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["extract"])
    ap.add_argument("--ual", default=os.environ.get("METALLIC_UAL"))
    a = ap.parse_args(argv)
    if not a.ual:
        raise SystemExit("--ual DIR (or METALLIC_UAL) is required")
    extract(a.ual)


if __name__ == "__main__":
    main()
