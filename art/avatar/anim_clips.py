"""The default avatar's clips and still poses, authored in code (Phase B).

Used by rig_avatar.py. Every clip is a function of the frame number that returns a pose: FK
rotations per bone (degrees, XYZ euler in the bone's local frame, see rig_avatar.py for the axes),
a world offset for the hips, and IK targets for the feet and, where a hand must hold a place, the
hands. rig_avatar.py solves the IK in Blender and bakes everything to FK keys.

Two kinds of clip:
- `Locomotion` (walk, run): procedural. The planted foot moves backward at exactly the controller's
  speed, so the feet cannot slide; stride and cycle time come from the speed (see the numbers in
  LOCOMOTION below).
- `Keyed`: key poses on chosen frames, interpolated per channel with clamped Catmull-Rom tangents
  (an extreme gets a flat tangent, like Blender's auto-clamped keys), cyclic when the clip loops.
  Per-bone lag (in frames) gives overlap and follow-through: the bone samples the curves later.

Names: VRM humanoid bone names. A pose is written for the left side and `mirror()` makes the right.
"""
import math

from mathutils import Euler, Matrix, Vector

import build_avatar as BA

FPS = 30
SK = BA.skeleton()
STUD = BA.STUD
FINGERS = ("Index", "Middle", "Ring", "Little")

ANKLE_L = SK["ANKLE"].copy()
HIP_L = SK["HIP"].copy()
# Ground contact points relative to the ankle (left foot, rest): heel and ball of the foot, and
# the toe tip (it turns with the toes bone, about the ball).
HEEL_REL = Vector((SK["HEEL"].x, SK["HEEL"].y, 0.0)) - ANKLE_L
BALL_REL = Vector((SK["BALL"].x, SK["BALL"].y, 0.0)) - ANKLE_L
TOE_REL = Vector((SK["TOE"].x, SK["TOE"].y, 0.0)) - ANKLE_L
LEG_LEN = (SK["KNEE"] - SK["HIP"]).length + (SK["ANKLE"] - SK["KNEE"]).length


# ======================================================================== pose helpers

def other(name):
    if name.startswith("left"):
        return "right" + name[4:]
    if name.startswith("right"):
        return "left" + name[5:]
    return name


def mirror_rot(rot):
    """Mirror a rotation dict: left <-> right, (x, y, z) -> (x, -y, -z)."""
    return {other(b): (r[0], -r[1], -r[2]) for b, r in rot.items()}


def side_rot(rot, side):
    """rot written for the left side, returned for `side`."""
    return dict(rot) if side == "left" else mirror_rot(rot)


def add(rot, more):
    out = dict(rot)
    for b, r in more.items():
        if b in out:
            out[b] = tuple(a + c for a, c in zip(out[b], r))
        else:
            out[b] = tuple(r)
    return out


def hand(shape, side="left", k=1.0):
    """Finger and thumb rotations for a hand shape, blended toward rest by k (1 = full shape)."""
    S = {
        # name: per finger (proximal, intermediate, distal) curl, spread; thumb (meta xyz, prox x, dist x)
        "relaxed": ({"Index": (30, 38, 20, 1), "Middle": (36, 44, 24, 0), "Ring": (42, 48, 26, -1),
                     "Little": (50, 52, 28, -2)}, ((2, 0, -20), 10, 12)),
        "loose": ({"Index": (52, 62, 34, 0), "Middle": (58, 68, 36, 0), "Ring": (62, 70, 38, 0),
                   "Little": (66, 72, 40, 0)}, ((4, 0, -36), 18, 20)),
        "fist": ({"Index": (88, 100, 62, 0), "Middle": (90, 102, 62, 0), "Ring": (92, 102, 62, 0),
                  "Little": (94, 102, 60, 0)}, ((5, 0, -52), 26, 26)),
        "open": ({"Index": (4, 4, 2, 7), "Middle": (2, 3, 2, 1), "Ring": (3, 4, 2, -5),
                  "Little": (6, 6, 3, -12)}, ((-6, 0, 10), 0, 4)),
        "flat": ({"Index": (1, 1, 1, 1), "Middle": (1, 1, 1, 0), "Ring": (1, 1, 1, -1),
                  "Little": (1, 1, 1, -2)}, ((0, 0, -4), 2, 2)),
        "point": ({"Index": (2, 2, 1, 2), "Middle": (88, 100, 62, 0), "Ring": (92, 102, 62, 0),
                   "Little": (94, 102, 60, 0)}, ((5, 0, -50), 26, 26)),
        "grip": ({"Index": (62, 72, 40, 0), "Middle": (64, 74, 40, 0), "Ring": (66, 74, 40, 0),
                  "Little": (68, 74, 40, 0)}, ((6, 0, -44), 22, 24)),
    }
    fingers, (meta, prox, dist) = S[shape]
    rot = {}
    for f, (p, i, d, spread) in fingers.items():
        rot[f"left{f}Proximal"] = (p * k, 0, spread * k)
        rot[f"left{f}Intermediate"] = (i * k, 0, 0)
        rot[f"left{f}Distal"] = (d * k, 0, 0)
    rot["leftThumbMetacarpal"] = tuple(x * k for x in meta)
    rot["leftThumbProximal"] = (prox * k, 0, 0)
    rot["leftThumbDistal"] = (dist * k, 0, 0)
    return side_rot(rot, side)


def hands(shape_l, shape_r=None, k=1.0):
    return {**hand(shape_l, "left", k), **hand(shape_r or shape_l, "right", k)}


# ======================================================================== IK targets

class Rig:
    """Rest data read from the armature once."""

    def __init__(self, arm):
        self.arm = arm
        b = arm.data.bones
        self.rest = {x.name: x.matrix_local.copy() for x in b}
        self.foot_rest = {s: self.rest[s + "Foot"].to_3x3() for s in ("left", "right")}
        self.hand_rest = {s: self.rest[s + "Hand"].to_3x3() for s in ("left", "right")}

    def hips_matrix(self, pose):
        r = pose.get("rot", {}).get("hips", (0, 0, 0))
        dx, dy, dz = pose.get("hips", (0, 0, 0))
        basis = Matrix.Translation((dx, dz, -dy)) @ Euler([math.radians(x) for x in r], "XYZ").to_matrix().to_4x4()
        return self.rest["hips"] @ basis

    def hip_joint(self, pose, side):
        s = 1 if side == "left" else -1
        m = self.hips_matrix(pose)
        rest_hip = Vector((HIP_L.x * s, HIP_L.y, HIP_L.z))
        return m @ (self.rest["hips"].inverted() @ rest_hip)


def foot_rot(rig, side, pitch=0.0, yaw=0.0, roll=0.0):
    """World rotation of the foot: pitch > 0 lifts the toe, yaw > 0 turns the toe outward,
    roll > 0 lifts the outer edge."""
    s = 1 if side == "left" else -1
    R = (Matrix.Rotation(math.radians(yaw * s), 3, "Z") @ Matrix.Rotation(math.radians(-pitch), 3, "X")
         @ Matrix.Rotation(math.radians(-roll * s), 3, "Y"))
    return R @ rig.foot_rest[side]


def hand_rot(rig, side, rx=0.0, ry=0.0, rz=0.0):
    """World rotation of the hand: an XYZ euler (degrees, world axes) applied to the rest hand,
    written for the left side and mirrored for the right."""
    s = 1 if side == "left" else -1
    e = Euler((math.radians(rx), math.radians(ry * s), math.radians(rz * s)), "XYZ").to_matrix()
    return e @ rig.hand_rest[side]


def hand_aim(rig, side, along, palm):
    """World rotation of the hand bone from two left-side directions: `along` = from the wrist toward
    the knuckles, `palm` = the way the palm faces. Mirrored in X for the right hand."""
    s = 1 if side == "left" else -1
    a = Vector((along[0] * s, along[1], along[2])).normalized()
    p = Vector((palm[0] * s, palm[1], palm[2]))
    c = (p - a * p.dot(a)).normalized()
    x = a.cross(c)
    return Matrix((x, a, c)).transposed()


def world_foot_point(side, ankle, R, rig, rel):
    """World position of a point given relative to the ankle at rest (left-side vector)."""
    s = 1 if side == "left" else -1
    rel_s = Vector((rel.x * s, rel.y, rel.z))
    return ankle + R @ (rig.foot_rest[side].inverted() @ rig.foot_rest[side]) @ rel_s


def knee_pole(rig, pose, side, ankle, yaw=0.0, fwd=None):
    """Pole in front of the knee: halfway between hip and ankle, 0.5 m along the body's forward."""
    s = 1 if side == "left" else -1
    hip = rig.hip_joint(pose, side)
    if fwd is None:
        m = rig.hips_matrix(pose).to_3x3()
        fwd = (m @ Vector((0, 0, 1))).normalized()
    fwd = Matrix.Rotation(math.radians(yaw * s), 3, "Z") @ fwd
    return (hip + ankle) / 2 + fwd * 0.5


def elbow_pole(rig, pose, side, wrist, back=(0.0, 1.0, 0.0)):
    s = 1 if side == "left" else -1
    sh = Vector((SK["SHOULDER"].x * s, SK["SHOULDER"].y, SK["SHOULDER"].z)) + Vector(pose.get("hips", (0, 0, 0)))
    d = Vector((back[0] * s, back[1], back[2])).normalized()
    return (sh + wrist) / 2 + d * 0.45


def contact_points(rig_bones, side):
    """World heel, ball and toe-tip sole points from a posed armature (slip and floor checks)."""
    pb = rig_bones[side + "Foot"]
    s = 1 if side == "left" else -1
    ankle = Vector((ANKLE_L.x * s, ANKLE_L.y, ANKLE_L.z))
    heel = ankle + Vector((HEEL_REL.x * s, HEEL_REL.y, HEEL_REL.z))
    ball = ankle + Vector((BALL_REL.x * s, BALL_REL.y, BALL_REL.z))
    toe = ankle + Vector((TOE_REL.x * s, TOE_REL.y, TOE_REL.z))
    m = pb.matrix @ pb.bone.matrix_local.inverted()
    tb = rig_bones[side + "Toes"]
    mt = tb.matrix @ tb.bone.matrix_local.inverted()
    return m @ heel, m @ ball, mt @ toe


# ======================================================================== interpolation

def clamped_cr(ts, vs, t, cyclic, period):
    """Cubic Hermite through (ts, vs) with Catmull-Rom tangents, flat at local extremes. Cyclic
    curves wrap with the given period (keys must lie in [0, period))."""
    n = len(ts)
    if n == 1:
        return vs[0]
    if cyclic:
        t = t % period
        T = [ts[-2] - period, ts[-1] - period] + list(ts) + [ts[0] + period, ts[1] + period]
        V = [vs[-2], vs[-1]] + list(vs) + [vs[0], vs[1]]
    else:
        if t <= ts[0]:
            return vs[0]
        if t >= ts[-1]:
            return vs[-1]
        T, V = list(ts), list(vs)
    i = max(k for k in range(len(T) - 1) if T[k] <= t)

    def tangent(k):
        if k <= 0 or k >= len(T) - 1:
            return 0.0
        vp, vk, vn = V[k - 1], V[k], V[k + 1]
        if (vk - vp) * (vn - vk) <= 0:
            return 0.0
        return (vn - vp) / (T[k + 1] - T[k - 1])

    t0, t1, v0, v1 = T[i], T[i + 1], V[i], V[i + 1]
    m0, m1 = tangent(i), tangent(i + 1)
    h = t1 - t0
    u = (t - t0) / h
    h00, h10, h01, h11 = 2 * u ** 3 - 3 * u ** 2 + 1, u ** 3 - 2 * u ** 2 + u, -2 * u ** 3 + 3 * u ** 2, u ** 3 - u ** 2
    return h00 * v0 + h10 * h * m0 + h01 * v1 + h11 * h * m1


def smooth(x):
    x = max(0.0, min(1.0, x))
    return x * x * (3 - 2 * x)


# ======================================================================== clips

class Clip:
    def __init__(self, name, length, loop, priority, export=True, speed=None, meta=None):
        self.name, self.length, self.loop, self.priority = name, length, loop, priority
        self.export, self.speed, self.meta = export, speed, dict(meta or {})

    def planted(self, side, f):
        return False


class Keyed(Clip):
    """Key poses. A key is (frame, spec) where spec has:
      rot   {bone: (x, y, z)}            FK rotations (missing bones = the clip's base)
      hips  (dx, dy, dz)                 world offset of the hips
      feet  {side: (x, y, z, pitch, yaw[, roll])}   foot IK target (ankle), world, written with
                                         left-side numbers; "right" mirrors "left" when not given
      hands {side: (x, y, z, ax, ay, az, px, py, pz)}  hand IK target: wrist position, the
                                         direction of the hand and the way the palm faces
      ikw   {chain: weight}              IK weights (feet given = 1 unless set)
      pole  {"elbow": (x, y, z)}         direction the elbows point (left side), default back
    """

    def __init__(self, rig, name, length, loop, keys, base=None, lag=None, priority=1, export=True,
                 meta=None, feet_mirror=True):
        super().__init__(name, length, loop, priority, export, meta=meta)
        self.rig = rig
        self.feet_mirror = feet_mirror
        self.base = base or {}
        self.lag = lag or {}
        keys = sorted(keys, key=lambda k: k[0])
        if loop:
            keys = [k for k in keys if k[0] < length] or keys[:1]
        self.keys = keys
        chans = {}
        for f, spec in self.keys:
            for ch, v in self.flatten(spec).items():
                chans.setdefault(ch, []).append((f, v))
        # A channel missing from a key takes the base value, or else the previous key's value.
        self.chans = {}
        allf = [f for f, _ in self.keys]
        base_flat = self.flatten(self.base)
        for ch, fv in chans.items():
            d = dict(fv)
            vals = []
            last = base_flat.get(ch, fv[0][1])
            for f in allf:
                if f in d:
                    last = d[f]
                elif ch in base_flat:
                    last = base_flat[ch]
                vals.append(last)
            self.chans[ch] = (allf, vals)
        for ch, v in base_flat.items():
            if ch not in self.chans:
                self.chans[ch] = ([0], [v])

    def flatten(self, spec):
        out = {}
        for b, r in spec.get("rot", {}).items():
            for i in range(3):
                out[("rot", b, i)] = r[i]
        for i, v in enumerate(spec.get("hips", ())):
            out[("hips", i)] = v
        feet = dict(spec.get("feet", {}))
        if self.feet_mirror and "left" in feet and "right" not in feet:
            feet["right"] = feet["left"]
        for side, v in feet.items():
            v = tuple(v) + (0.0,) * (6 - len(v))
            for i in range(6):
                out[("foot", side, i)] = v[i]
        for side, v in spec.get("hands", {}).items():
            for i in range(9):
                out[("hand", side, i)] = v[i]
        for ch, v in spec.get("ikw", {}).items():
            out[("ikw", ch)] = v
        for ch, v in spec.get("pole", {}).items():
            for i in range(len(v)):
                out[("pole", ch, i)] = v[i]
        return out

    def lag_of(self, ch):
        key = ch[1] if ch[0] == "rot" else ch[0]
        for k, l in self.lag.items():
            if key == k or (k.endswith("*") and key.startswith(k[:-1])) or \
                    (k.startswith("*") and key.endswith(k[1:])) or \
                    ("*" in k[1:-1] and key.startswith(k.split("*")[0]) and key.endswith(k.split("*")[1])):
                return l
        return 0.0

    def value(self, ch, f):
        ts, vs = self.chans[ch]
        return clamped_cr(ts, vs, f - self.lag_of(ch), self.loop, self.length)

    def sample(self, f):
        vals = {ch: self.value(ch, f) for ch in self.chans}
        return build_pose(self.rig, vals)


def build_pose(rig, vals):
    """Turn flat channel values into a pose for rig_avatar.apply_pose."""
    pose = {"rot": {}, "hips": (0, 0, 0), "ik": {}, "ikw": {}}
    for ch, v in vals.items():
        if ch[0] == "rot":
            r = list(pose["rot"].get(ch[1], (0, 0, 0)))
            r[ch[2]] = v
            pose["rot"][ch[1]] = tuple(r)
        elif ch[0] == "ikw":
            pose["ikw"][ch[1]] = max(0.0, min(1.0, v))
    pose["hips"] = tuple(vals.get(("hips", i), 0.0) for i in range(3))
    for side in ("left", "right"):
        s = 1 if side == "left" else -1
        if ("foot", side, 0) in vals:
            x, y, z, pitch, yaw, roll = [vals[("foot", side, i)] for i in range(6)]
            ankle = Vector((x * s, y, z))
            pose["ikw"].setdefault(f"leg.{side}", 1.0)
            pose["ik"][f"leg.{side}"] = (ankle, foot_rot(rig, side, pitch, yaw, roll),
                                         knee_pole(rig, pose, side, ankle, vals.get(("pole", "knee", 0), yaw)))
        if ("hand", side, 0) in vals:
            v = [vals[("hand", side, i)] for i in range(9)]
            wrist = Vector((v[0] * s, v[1], v[2]))
            back = tuple(vals.get(("pole", "elbow", i), d) for i, d in enumerate((0.3, 1.0, 0.0)))
            pose["ikw"].setdefault(f"arm.{side}", 1.0)
            pose["ik"][f"arm.{side}"] = (wrist, hand_aim(rig, side, v[3:6], v[6:9]),
                                         elbow_pole(rig, pose, side, wrist, back))
    return pose


def merge(*dicts):
    out = {}
    for d in dicts:
        out.update(d)
    return out


def flat_rot(rot):
    return {("rot", b, i): r[i] for b, r in rot.items() for i in range(3)}


class Proc(Clip):
    """A clip whose pose comes from a function of the frame."""

    def __init__(self, name, length, fn, priority, meta=None):
        super().__init__(name, length, True, priority, meta=meta)
        self.fn = fn

    def sample(self, f):
        return self.fn(f)


# ======================================================================== locomotion

LOCOMOTION = {
    # Controller: WalkSpeed 16 studs/s, run = 1.5 x 16 = 24 studs/s, 1 stud = 0.28 m, figure 5.5 studs.
    # The figure's leg (hip joint to ankle) is 0.64 m, so 4.48 m/s is already a jog (Froude number
    # v^2 / (g L) = 3.2; people switch from walking to running near 0.5). A planted foot can cover
    # at most about 0.8 m of ground before the leg runs out of reach, so the stance must be short and
    # both gaits need a flight phase. Roblox does the same: its Animate script plays the run clip
    # at WalkSpeed 16.
    # A true walk for partial stick / touch input (the engine blends it with "walk" by speed). With
    # this leg a walk (no flight, Froude number under 0.5) tops out near 6 studs/s.
    "walk_slow": dict(studs=6.0, frames=20, stance_m=0.60, mid=-0.03, track=0.10, toe_out=6.0,
                      heel=20.0, toe=30.0, lift=0.06, lift_peak=0.35, drop=-0.020, bob=0.018, down=0.04,
                      sway=0.022, pelvis_yaw=6.0, pelvis_roll=4.0, lean=2.0, chest_yaw=5.0,
                      arm=18.0, arm_base=2.0, elbow=(14.0, 8.0), arm_in=-4.0, hand="relaxed", lag=0.05),
    "walk": dict(studs=16.0, frames=16, stance_m=0.66, mid=-0.04, track=0.122, toe_out=5.0,
                 heel=14.0, toe=34.0, lift=0.11, lift_peak=0.40, drop=-0.025, bob=0.018,
                 sway=0.010, pelvis_yaw=8.0, pelvis_roll=3.0, lean=5.0, chest_yaw=7.0,
                 arm=26.0, arm_base=4.0, elbow=(30.0, 22.0), arm_in=-4.0, hand="loose", lag=0.045),
    "run": dict(studs=24.0, frames=16, stance_m=0.68, mid=-0.08, track=0.100, toe_out=3.0,
                heel=5.0, toe=44.0, lift=0.22, lift_peak=0.34, drop=-0.032, bob=0.026,
                sway=0.006, pelvis_yaw=13.0, pelvis_roll=2.0, lean=23.0, chest_yaw=17.0,
                arm=58.0, arm_base=12.0, elbow=(88.0, 20.0), arm_in=-8.0, hand="fist", lag=0.04,
                shoulder=11.0),
}


# Heel strike to foot flat, as a fraction of the gait cycle: the same in every gait (#280).
ROLL_IN = 0.07


class Locomotion(Clip):
    """In-place walk / run cycle. Phase 0 = left heel strike. Each foot: a stance phase where the
    ground under it moves backward at exactly the controller's speed (heel strike, foot flat, roll
    over the ball, toe-off) and a swing phase (cubic path whose end speeds match the ground)."""

    def __init__(self, rig, name, p):
        self.p = p
        self.v = p["studs"] * STUD
        self.N = p["frames"]
        self.T = self.N / FPS
        self.beta = p["stance_m"] / (self.v * self.T)     # stance as a fraction of the cycle
        meta = {"speed_studs_s": p["studs"], "speed_m_s": round(self.v, 3), "cycle_frames": self.N,
                "cycle_s": round(self.T, 4), "stride_m": round(self.v * self.T, 4),
                "stride_studs": round(self.v * self.T / STUD, 3), "step_m": round(self.v * self.T / 2, 4),
                "cadence_steps_min": round(120 / self.T, 1), "stance_frames": round(self.beta * self.N, 2),
                "flight_frames_per_step": round(max(0.0, 0.5 - self.beta) * self.N, 2),
                "double_support_frames_per_step": round(max(0.0, self.beta - 0.5) * self.N, 2),
                "ground_per_stance_m": p["stance_m"], "left_contact_frame": 0, "right_contact_frame": self.N // 2}
        super().__init__(name, self.N, True, 1, True, speed=self.v, meta=meta)
        self.rig = rig
        self.max_reach = 0.0

    def upper_body(self, f, rot):
        """Hook for the trunk, arms and head (RetargetLoco takes them from a source clip)."""
        return rot

    def stance_state(self, u):
        p = self.p
        A = p["mid"] + p["stance_m"] / 2 - p["stance_m"] * u      # flat-foot ankle, forward coordinate
        # Heel to flat takes the same fraction of the cycle in every gait (ROLL_IN, issue #280): the
        # runtime blends gaits at one shared phase, and a foot that rolls in at different rates in
        # the two clips turns about the heel below the ground in the mix and slides.
        u1, u2 = min(0.45, ROLL_IN / self.beta), 0.48
        if u < u1:
            pitch = p["heel"] * (1 - smooth(u / u1))
        elif u < u2:
            pitch = 0.0
        else:
            pitch = -p["toe"] * ((u - u2) / (1 - u2)) ** 1.5
        return self.ankle_from(A, pitch), pitch, max(0.0, -pitch)

    def ankle_from(self, A, pitch):
        """Ankle position for a foot whose flat footprint has its ankle at forward coordinate A,
        pitched about the heel (pitch > 0) or the ball (pitch < 0)."""
        Rz = Matrix.Rotation(math.radians(self.p["toe_out"]), 3, "Z")
        Rx = Matrix.Rotation(math.radians(-pitch), 3, "X")
        flat = Vector((self.p["track"], -A, ANKLE_L.z))
        rel = HEEL_REL if pitch > 0 else BALL_REL
        pivot = flat + Rz @ rel
        return pivot + Rz @ Rx @ (-rel)

    def foot(self, phase):
        b = self.beta
        if phase < b:
            ank, pitch, toes = self.stance_state(phase / b)
            return ank, pitch, toes, True
        w = (phase - b) / (1 - b)
        a0, p0, t0 = self.stance_state(1.0)
        a1, p1, _ = self.stance_state(0.0)
        dt = (1 - b) * self.T
        s0, s1 = -a0.y, -a1.y
        # The swinging foot follows through a little behind, then the knee drives it forward early
        # (time warp w^0.8) and it sweeps back slightly into the next contact.
        m0, m1 = -self.v * dt * 0.15, -self.v * dt * 0.30
        ws = w ** 0.8
        h00, h10, h01, h11 = 2 * ws ** 3 - 3 * ws ** 2 + 1, ws ** 3 - 2 * ws ** 2 + ws, -2 * ws ** 3 + 3 * ws ** 2, ws ** 3 - ws ** 2
        s = h00 * s0 + h10 * m0 + h01 * s1 + h11 * m1
        pk = self.p["lift_peak"]
        bump = math.sin(math.pi * (w ** (math.log(0.5) / math.log(pk)))) ** 1.3
        z = a0.z + (a1.z - a0.z) * smooth(w) + self.p["lift"] * bump
        x = a0.x + (a1.x - a0.x) * w
        pitch = clamped_cr([0.0, 0.25, 0.7, 1.0], [p0, p0 * 0.8, self.p["heel"] * 0.5, self.p["heel"]], w, False, 1)
        toes = t0 * (1 - smooth(w / 0.35))
        return Vector((x, -s, z)), pitch, toes, False

    def phase(self, side, f):
        return (f / self.N + (0.0 if side == "left" else 0.5)) % 1.0

    def planted(self, side, f):
        return self.phase(side, f) < self.beta - 1e-6

    def sample(self, f):
        p, rig = self.p, self.rig
        ph = f / self.N
        two = 2 * math.pi
        # Hips: lowest just after contact ("down"), highest in the flight ("up").
        down = p.get("down", 0.45 * self.beta)
        dz = p["drop"] - p["bob"] * math.cos(2 * two * (ph - down))
        dx = p["sway"] * math.cos(two * (ph - self.beta / 2))
        vals = {("hips", 0): dx, ("hips", 1): 0.0, ("hips", 2): dz}
        yaw = -p["pelvis_yaw"] * math.cos(two * (ph - 0.02))
        roll = p["pelvis_roll"] * math.cos(two * (ph - down))
        rot = {"hips": (p["lean"] * 0.55, yaw, roll)}
        cy = p["chest_yaw"] * math.cos(two * (ph - 0.06))
        rot["spine"] = (p["lean"] * 0.25, cy * 0.4, -roll * 0.5)
        rot["chest"] = (p["lean"] * 0.2, cy * 0.5, -roll * 0.4)
        rot["upperChest"] = (0.0, cy * 0.6, -roll * 0.2)
        # Head: stays level and looks ahead, catching up a little late (overlap).
        lag = p["lag"]
        yaw_l = -p["pelvis_yaw"] * math.cos(two * (ph - 0.02 - lag * 2)) + \
            1.5 * p["chest_yaw"] * math.cos(two * (ph - 0.06 - lag * 2))
        bob_l = math.cos(2 * two * (ph - down - lag))
        rot["neck"] = (-p["lean"] * 0.3, -yaw_l * 0.45, 0.0)
        rot["head"] = (-p["lean"] * 0.3 + 1.5 * bob_l, -yaw_l * 0.45, -roll * 0.3)
        # Arms swing against the legs: the left arm is forward when the right foot is.
        for side in ("left", "right"):
            sp = self.phase(side, f)
            arm = {}
            c1 = math.cos(two * (sp - 0.5 - lag))
            c2 = math.cos(two * (sp - 0.5 - 2.2 * lag))
            c3 = math.cos(two * (sp - 0.5 - 3.5 * lag))
            arm["leftUpperArm"] = (p["arm_base"] + p["arm"] * c1, -6.0 * c1, p["arm_in"])
            e0, e1 = p["elbow"]
            arm["leftLowerArm"] = (e0 + e1 * c2, 0.0, 0.0)
            arm["leftHand"] = (8.0 - 8.0 * c3, -10.0, 0.0)
            arm["leftShoulder"] = (1.5 * math.cos(2 * two * (ph - down - lag)), 0.0, -p.get("shoulder", 5.0) * c1)
            arm.update(hand(p["hand"]))
            rot.update(side_rot(arm, side))
        rot = self.upper_body(f, rot)
        vals.update(flat_rot(rot))
        pose = build_pose(rig, vals)
        # A planted foot must be reachable: when the hips' rotation (from the source) lifts a hip
        # joint too far, the hips come down instead of the knee locking and the foot sliding.
        drop = 0.0
        for side in ("left", "right"):
            if self.planted(side, f):
                ank, *_ = self.foot(self.phase(side, f))
                ank = Vector((ank.x * (1 if side == "left" else -1), ank.y, ank.z))
                hip = rig.hip_joint(pose, side)
                d = ank - hip
                reach = 0.98 * LEG_LEN
                if d.length > reach:
                    drop = max(drop, (hip.z - ank.z) - math.sqrt(max(0.0, reach * reach - d.x * d.x - d.y * d.y)))
        if drop > 0:
            vals[("hips", 2)] -= drop
            pose = build_pose(rig, vals)
        for side in ("left", "right"):
            ank, pitch, toes, _ = self.foot(self.phase(side, f))
            s = 1 if side == "left" else -1
            ank = Vector((ank.x * s, ank.y, ank.z))
            if not self.planted(side, f):
                # A swinging foot never asks for more leg than there is (no IK snap).
                hip = rig.hip_joint(pose, side)
                d = ank - hip
                if d.length > 0.975 * LEG_LEN:
                    ank = hip + d.normalized() * 0.975 * LEG_LEN
            pose["ikw"][f"leg.{side}"] = 1.0
            pose["ik"][f"leg.{side}"] = (ank, foot_rot(rig, side, pitch, p["toe_out"]),
                                         knee_pole(rig, pose, side, ank, p["toe_out"]))
            pose["rot"][side + "Toes"] = (toes, 0.0, 0.0)
            reach = (rig.hip_joint(pose, side) - ank).length / LEG_LEN
            self.max_reach = max(self.max_reach, reach)
        self.meta["max_leg_reach"] = round(self.max_reach, 4)
        return pose


LEG_BONES = ("UpperLeg", "LowerLeg", "Foot", "Toes")


def source_contacts(R, clip):
    """Left and right heel-strike frames of an in-place source loop: after the ankle's most
    forward point, the first frame where it is within 1.5 cm of its lowest height."""
    sc = R.source.clip(R.pack, clip)
    n = sc.last
    out = {}
    for side in ("left", "right"):
        pts = [sc.frames[i][side + "Foot"][1] for i in range(n)]
        zmin = min(p.z for p in pts)
        fwd = min(range(n), key=lambda i: pts[i].y)
        hit = fwd
        for k in range(n // 2):
            i = (fwd + k) % n
            if pts[i].z <= zmin + 0.015:
                hit = i
                break
        out[side] = float(hit)
    return out["left"], out["right"], n


def phase_map(tl, tr, n, frames):
    """Target frame -> source frame so the source's left heel strike lands on phase 0 and its
    right on phase 0.5 (piecewise linear between the two contacts)."""
    d1 = (tr - tl) % n
    d2 = n - d1

    def m(f):
        ph = (f / frames) % 1.0
        if ph < 0.5:
            return tl + d1 * ph / 0.5
        return tl + d1 + d2 * (ph - 0.5) / 0.5
    return m


class RetargetLoco(Locomotion):
    """A gait whose legs are the procedural, exactly planted cycle of `Locomotion` (so the
    runtime's speed matching and the shared phase hold) and whose hips rotation, trunk, arms,
    hands and head come from a retargeted source cycle, phase-locked to the legs."""

    def __init__(self, rig, name, p, R, src, lean=0.0, **polish):
        super().__init__(rig, name, p)
        tl, tr, n = source_contacts(R, src)
        import retarget_clips as RC
        self.inner = RC.RetargetClip(rig, R, name + "_upper", src, self.N, True,
                                     timemap=phase_map(tl, tr, n, self.N), feet="fk", **polish)
        self.lean = lean
        self.meta["source"] = self.inner.meta["source"] + " (trunk, arms, head); legs procedural"
        self.meta["source_contacts"] = [tl, tr, n]
        self.meta["roll_in_cycle"] = ROLL_IN

    def upper_body(self, f, rot):
        src = self.inner.sample(f)["rot"]
        out = dict(rot)
        for b, r in src.items():
            if b == "root" or b.endswith(LEG_BONES):
                continue
            out[b] = r
        x, y, z = out["hips"]
        out["hips"] = (x + self.lean, y, z)
        return out


# ======================================================================== shared poses

def stand_feet(toe_out=12.0, x=0.160, y=0.012):
    """Feet planted a little inside the rest ankles, toes turned out (the concept's stance)."""
    return {"left": (x, ANKLE_L.y + y, ANKLE_L.z, 0.0, toe_out)}


def arms_down(k=1.0, side=None):
    rot = {"leftUpperArm": (4 * k, 0, -8 * k), "leftLowerArm": (16 * k, 0, 0), "leftHand": (6 * k, -14 * k, 0)}
    return side_rot(rot, side) if side else {**rot, **mirror_rot(rot)}


IDLE_BASE = {
    "rot": merge(arms_down(), hands("relaxed"), {"leftShoulder": (0, 0, 0), "rightShoulder": (0, 0, 0)}),
    "hips": (0, 0, -0.012),
    "feet": stand_feet(),
}


def idle_key(hx, hz, hip_rot, spine, chest, uc, neck, head, breath=0.0, arm_k=1.0, extra=None):
    rot = {"hips": hip_rot, "spine": spine, "chest": chest, "upperChest": (uc[0] - 2.0 * breath, uc[1], uc[2]),
           "neck": neck, "head": head,
           "leftShoulder": (1.6 * breath, 0, 0), "rightShoulder": (1.6 * breath, 0, 0)}
    rot.update(arms_down(arm_k))
    rot.update(hands("relaxed"))
    if extra:
        rot = add(rot, extra)
    return {"rot": rot, "hips": (hx, 0.0, hz + 0.004 * breath), "feet": stand_feet()}


IDLE_LAG = {"left*Arm": 3, "right*Arm": 3, "leftHand": 5, "rightHand": 5, "neck": 2, "head": 4}


def idle_start():
    return idle_key(0.016, -0.012, (0, -3, 3.0), (1, 1, -1.6), (0.5, 1, -1.6), (-0.5, 0, 0), (-1, 0, 1), (-1, 3, 1.5))


def idle_clips(rig):
    out = []
    # Idle A, 3 s: weight on the left leg (contrapposto: that hip up, shoulders counter-tilted),
    # two slow breaths, a small drift of the weight and the head. Arms, hands and head lag the body.
    keys = [
        (0, idle_start()),
        (22, idle_key(0.019, -0.010, (0, -3.5, 3.4), (1, 1.2, -1.8), (0.2, 1.2, -1.8), (-0.5, 0, 0), (-1.5, 0, 1), (-2, 4, 2.0), 1.0)),
        (45, idle_key(0.020, -0.013, (0, -2.5, 3.6), (1, 0.8, -2.0), (0.6, 0.8, -1.8), (-0.5, 0, 0), (-1, 0, 1.2), (-0.5, 1, 2.2), 0.0)),
        (67, idle_key(0.017, -0.011, (0, -3, 3.2), (1, 1, -1.7), (0.3, 1, -1.6), (-0.5, 0, 0), (-1.5, 0, 0.8), (-2, 2.5, 1.4), 1.0)),
    ]
    out.append(Keyed(rig, "idle", 90, True, keys, base=IDLE_BASE, lag=IDLE_LAG, priority=1))

    # Idle B, 4 s: shifts the weight to the right leg, looks left, then right, and settles.
    kb = [
        (0, idle_start()),
        (20, idle_key(-0.004, -0.016, (0, 0, 0.0), (1, 0, 0), (0.5, 0, 0), (-0.5, 0, 0), (-1, 6, 0), (-1, 10, 0), 0.6)),
        (36, idle_key(-0.016, -0.012, (0, 3, -3.0), (1, -1, 1.6), (0.5, 2, 1.6), (-0.5, 4, 0), (-2, 14, -1), (-3, 18, -2), 1.0)),
        (58, idle_key(-0.018, -0.012, (0, 3, -3.2), (1, -1, 1.6), (0.5, 2, 1.6), (-0.5, 4, 0), (-2, 15, -1), (-3, 20, -3), 0.2)),
        (74, idle_key(-0.015, -0.012, (0, 2, -3.0), (1, -2, 1.6), (0.5, -2, 1.6), (-0.5, -4, 0), (0, -14, 1), (1, -18, 2), 0.8)),
        (96, idle_key(-0.016, -0.012, (0, 2, -3.0), (1, -2, 1.6), (0.5, -2, 1.6), (-0.5, -4, 0), (0, -15, 1), (1, -19, 2), 0.1)),
        (108, idle_key(0.004, -0.015, (0, -1, 1.0), (1, 0, -0.5), (0.5, 0, -0.5), (-0.5, 0, 0), (-1, 0, 0.5), (-1, 0, 1), 0.5)),
    ]
    out.append(Keyed(rig, "idle_b", 120, True, kb, base=IDLE_BASE, lag=IDLE_LAG, priority=2))
    return out


def body_pose(hips_p, spine_p, chest_p, neck_p, head_p, arm_x, arm_z, elbow, hand_x=0.0, shape="relaxed"):
    rot = {"hips": (hips_p, 0, 0), "spine": (spine_p, 0, 0), "chest": (chest_p, 0, 0),
           "upperChest": (chest_p * 0.5, 0, 0), "neck": (neck_p, 0, 0), "head": (head_p, 0, 0)}
    arm = {"leftUpperArm": (arm_x, 0, arm_z), "leftLowerArm": (elbow, 0, 0), "leftHand": (hand_x, -10, 0)}
    rot.update(arm)
    rot.update(mirror_rot(arm))
    rot.update(hands(shape))
    return rot


def jump_clips(rig):
    out = []
    F = stand_feet()
    lag = {"left*Arm": 1, "right*Arm": 1, "leftHand": 2, "rightHand": 2, "head": 2, "neck": 1}
    body = body_pose

    # Jump: 4 frames of anticipation (down, arms back), launch on frame 6 (legs extend, arms throw
    # up), then the tuck rising into the air; the last frame hands over to the fall loop.
    keys = [
        (0, {"rot": body(0, 0, 0, 0, 0, 4, -8, 16), "hips": (0, 0, -0.012), "feet": F}),
        (4, {"rot": body(24, 8, 6, -18, -10, -38, -2, 30, 10), "hips": (0, 0.03, -0.135), "feet": F}),
        (6, {"rot": body(6, 0, -2, -4, -4, 70, 8, 30, -6, "open"), "hips": (0, 0.0, 0.02),
             "feet": {"left": (0.150, ANKLE_L.y + 0.04, 0.16, -42, 10)}}),
        (9, {"rot": body(-6, -4, -4, 2, 4, 22, 132, 18, -10, "open"), "hips": (0, 0, 0.03),
             "feet": {"left": (0.140, ANKLE_L.y - 0.06, 0.30, -20, 8), "right": (0.150, ANKLE_L.y + 0.08, 0.20, -34, 8)}}),
        (14, {"rot": body(-2, -2, -2, 2, 2, 18, 122, 26, -8, "open"), "hips": (0, 0, 0.02),
              "feet": {"left": (0.140, ANKLE_L.y - 0.10, 0.34, -12, 8), "right": (0.150, ANKLE_L.y + 0.04, 0.24, -26, 8)}}),
        (18, {"rot": body(0, -2, -2, 0, 2, 20, 100, 30, -6, "open"), "hips": (0, 0, 0.01),
              "feet": {"left": (0.145, ANKLE_L.y - 0.07, 0.28, -16, 8), "right": (0.150, ANKLE_L.y + 0.05, 0.22, -24, 8)}}),
    ]
    out.append(Keyed(rig, "jump", 18, False, keys, lag=lag, priority=1, meta={"launch_frame": 6,
                     "note": "frames 0-5 are the anticipation; apply the jump impulse on frame 6"}))

    # Fall loop, 0.8 s: arms up and out, open hands paddling, legs dangling and treading slowly.
    def fall(arm_off, leg_off, head):
        rot = {"hips": (-4, 0, 0), "spine": (-3, 0, 0), "chest": (-2, 0, 0), "neck": (4, 0, 0), "head": (head, 0, 0)}
        la = {"leftUpperArm": (14 + arm_off, 0, 100 - arm_off), "leftLowerArm": (30 - arm_off * 0.5, 0, 0),
              "leftHand": (-12, -10, 0), "leftShoulder": (6, 0, 0)}
        ra = {"leftUpperArm": (14 - arm_off, 0, 100 + arm_off), "leftLowerArm": (30 + arm_off * 0.5, 0, 0),
              "leftHand": (-12, -10, 0), "leftShoulder": (6, 0, 0)}
        rot.update(la)
        rot.update(mirror_rot(ra))
        rot.update(hands("open"))
        feet = {"left": (0.150, ANKLE_L.y - 0.05 + leg_off, 0.20 + leg_off * 0.6, -30, 8),
                "right": (0.160, ANKLE_L.y + 0.06 - leg_off, 0.16 - leg_off * 0.6, -36, 8)}
        return {"rot": rot, "hips": (0, 0, 0.0), "feet": feet}

    keys = [(0, fall(6, 0.03, 2)), (12, fall(-6, -0.03, -1))]
    out.append(Keyed(rig, "fall", 24, True, keys, lag={"left*Arm": 2, "right*Arm": 2, "*Hand": 3, "head": 3},
                     priority=1, feet_mirror=False))

    # Land: feet reach down (frame 0 is the touchdown), the body sinks and absorbs (frame 3), springs
    # back past neutral and settles into idle's first pose.
    keys = [
        (0, {"rot": body(-2, -2, -2, 2, 2, 8, 104, 32, -10, "open"), "hips": (0, 0, 0.0), "feet": F}),
        (3, {"rot": body(26, 10, 6, -22, -12, 30, 30, 50, 10, "relaxed"), "hips": (0, 0.04, -0.17), "feet": F}),
        (7, {"rot": body(12, 4, 2, -8, -6, 12, 10, 34, 8), "hips": (0, 0.015, -0.07), "feet": F}),
        (11, {"rot": body(-1, 0, 0, 1, 1, 2, -10, 14, 4), "hips": (0, 0, 0.0), "feet": F}),
        (16, idle_start()),
    ]
    out.append(Keyed(rig, "land", 16, False, keys, lag=lag, priority=1))
    return out


def p2_clips(rig):
    out = []
    # ---- climb: a ladder or truss in front (-Y). Hands and feet hold rungs that move down at the
    # climb speed while held; the opposite hand and foot reach up together.
    N = 24
    rise = 0.56                       # metres per cycle (two rungs of 0.28 m)
    T = N / FPS
    vc = rise / T
    wall_y = -0.17
    hold = 0.55                       # fraction of the cycle a hand or foot holds on

    def held(ph, top):
        """Height of a held point: it moves down with the rungs, then reaches back up."""
        if ph < hold:
            return top - vc * ph * T, 0.0
        w = (ph - hold) / (1 - hold)
        low = top - vc * hold * T
        return low + (top - low) * smooth(w), math.sin(math.pi * w)

    def climb_sample(f):
        ph0 = f / N
        two = 2 * math.pi
        # The head looks up after the reaching hand (lagging it by ~0.1 of a cycle), the chest
        # twists toward it and the hips swing over the foot that holds.
        look = math.cos(two * (ph0 - 0.10))
        up = abs(math.sin(two * (ph0 - 0.10)))
        rot = {"hips": (2, 5 * math.cos(two * ph0), 3 * math.cos(two * ph0)), "spine": (2, 2 * math.cos(two * ph0), 0),
               "chest": (4, -7 * math.cos(two * (ph0 - 0.05)), 0), "neck": (-8 - 3 * up, 4 * look, 0),
               "head": (-14 - 6 * up, 9 * look, 3 * math.cos(two * (ph0 - 0.18)))}
        vals = {}
        for side in ("left", "right"):
            ph = (ph0 + (0 if side == "left" else 0.5)) % 1
            z, reach = held(ph, 1.42)
            # The reaching hand arcs out (away from the wall and to the side) and back in.
            vals.update({("hand", side, i): v for i, v in enumerate(
                (0.15 + 0.05 * reach, wall_y - 0.02 + 0.11 * reach, z - 0.06, 0.0, -0.25, 1.0, 0.0, -1.0, -0.15))})
            fz, lift = held((ph + 0.5) % 1, 0.40)
            vals.update({("foot", side, i): v for i, v in enumerate(
                (0.13, wall_y + 0.09 + 0.06 * lift, fz + 0.03 * lift, -6 + 10 * lift, 6, 0))})
            rot.update(side_rot(hand("grip"), side))
            rot.update(side_rot({"leftShoulder": (6 * reach, 0, -4)}, side))
        vals.update(flat_rot(rot))
        vals[("hips", 0)] = 0.026 * math.cos(two * (ph0 - 0.05))
        vals[("hips", 1)] = 0.08          # hips back so the knees stay behind the rungs
        vals[("hips", 2)] = -0.04 + 0.015 * math.cos(2 * two * ph0)
        vals[("pole", "elbow", 0)], vals[("pole", "elbow", 1)], vals[("pole", "elbow", 2)] = 1.0, 0.4, -0.6
        return build_pose(rig, vals)

    out.append(Proc("climb", N, climb_sample, 2,
                    meta={"climb_speed_m_s": round(vc, 3), "climb_speed_studs_s": round(vc / STUD, 2),
                          "rung_spacing_m": 0.28, "wall_offset_m": -wall_y}))

    # ---- swim: front crawl, body horizontal, 1.2 s per arm cycle, flutter kick at 3 beats per arm.
    N = 36

    def swim_sample(f):
        ph = f / N
        two = 2 * math.pi
        roll = 22 * math.sin(two * ph)
        rot = {"hips": (78, roll * 0.6, 0), "spine": (2, roll * 0.25, 0), "chest": (2, roll * 0.2, 0),
               "upperChest": (0, 0, 0), "neck": (-28, -roll * 0.4, 0), "head": (-26, -roll * 0.3, 0)}
        for side in ("left", "right"):
            sp = (ph + (0 if side == "left" else 0.5)) % 1
            # pull (0-0.5): the arm sweeps from overhead (X 165) down under the body to the hip;
            # recovery (0.5-1): elbow high, the arm swings forward over the water.
            if sp < 0.5:
                w = sp / 0.5
                ax = 165 - 180 * smooth(w)
                az = -10 + 6 * math.sin(math.pi * w)
                el = 10 + 50 * math.sin(math.pi * w)
                hx = 10 * math.sin(math.pi * w)
            else:
                w = (sp - 0.5) / 0.5
                ax = -15 + 180 * smooth(w)
                az = 15 + 45 * math.sin(math.pi * w)
                el = 15 + 70 * math.sin(math.pi * w)
                hx = -10 * math.sin(math.pi * w)
            arm = {"leftUpperArm": (ax, 0, az), "leftLowerArm": (el, 0, 0), "leftHand": (hx, -10, 0),
                   "leftShoulder": (4 * math.cos(two * sp), 0, -8 * math.cos(two * sp))}
            arm.update(hand("flat"))
            rot.update(side_rot(arm, side))
        vals = flat_rot(rot)
        vals[("hips", 2)] = -0.02 + 0.01 * math.cos(2 * two * ph)
        for side in ("left", "right"):
            k = math.sin(3 * two * ph + (0 if side == "left" else math.pi))
            vals.update({("foot", side, i): v for i, v in enumerate(
                (0.085, 0.60, 0.70 + 0.07 * k, -78 + 8 * k, 4, 0))})
        return build_pose(rig, vals)

    out.append(Proc("swim", N, swim_sample, 2, meta={"note": "body horizontal at hips height, facing -Y (glTF +Z)"}))

    # ---- swim idle: treading water, upright, sculling hands and an eggbeater kick, 1.6 s.
    N = 48

    def swim_idle_sample(f):
        ph = f / N
        two = 2 * math.pi
        rot = {"hips": (10, 0, 0), "spine": (2, 0, 0), "chest": (0, 0, 0), "neck": (-6, 0, 0),
               "head": (-6 + 2 * math.sin(two * ph), 0, 0)}
        for side in ("left", "right"):
            sc = math.sin(2 * two * ph + (0 if side == "left" else 0.4))
            arm = {"leftUpperArm": (22, 6 * sc, 48 + 4 * sc), "leftLowerArm": (38 - 6 * sc, 0, 0),
                   "leftHand": (-8, -28 * sc - 10, 0), "leftShoulder": (2, 0, 0)}
            arm.update(hand("flat"))
            rot.update(side_rot(arm, side))
        vals = flat_rot(rot)
        vals[("hips", 2)] = -0.06 + 0.018 * math.sin(2 * two * ph)
        for side in ("left", "right"):
            a = two * ph + (0 if side == "left" else math.pi)
            vals.update({("foot", side, i): v for i, v in enumerate(
                (0.17 + 0.03 * math.cos(a), 0.10 + 0.07 * math.sin(a), 0.30 + 0.06 * math.cos(a),
                 -30 + 10 * math.sin(a), 18, 0))})
        return build_pose(rig, vals)

    out.append(Proc("swim_idle", N, swim_idle_sample, 2))

    # ---- sit: on a seat 0.40 m high (thighs level, feet on the ground), hands on the thighs,
    # breathing. 2 s loop.
    def sit_key(breath, head_yaw):
        rot = {"hips": (-4, 0, 0), "spine": (6, 0, 0), "chest": (6, 0, 0), "upperChest": (2 - 2 * breath, 0, 0),
               "neck": (-4, head_yaw * 0.4, 0), "head": (-2, head_yaw * 0.6, 0),
               "leftShoulder": (1.5 * breath, 0, 0), "rightShoulder": (1.5 * breath, 0, 0)}
        rot.update(hands("relaxed", k=0.7))
        h = (0.135, -0.235, 0.53, 0.15, -0.95, -0.2, -0.1, 0.0, -1.0)
        return {"rot": rot, "hips": (0, 0.05, -0.30 + 0.003 * breath),
                "feet": {"left": (0.155, -0.30, ANKLE_L.z, 0.0, 8.0)},
                "hands": {"left": h, "right": h}, "pole": {"elbow": (1.0, 0.6, 0.0)}}

    keys = [(0, sit_key(0, 0)), (30, sit_key(1, 4))]
    out.append(Keyed(rig, "sit", 60, True, keys, lag={"head": 4, "neck": 3}, priority=2,
                     meta={"seat_height_m": 0.40, "note": "feet on the ground plane, thighs level at 0.40 m"}))
    return out


def emote_clips(rig):
    out = []
    lag = {"left*Arm": 2, "right*Arm": 2, "leftHand": 3, "rightHand": 3, "head": 3, "neck": 2}
    start = idle_start()

    def with_arm(key, side, arm, shape=None, extra=None):
        k = {"rot": dict(key["rot"]), "hips": key["hips"], "feet": key["feet"]}
        k["rot"].update(side_rot(arm, side))
        if shape:
            k["rot"].update(hand(shape, side))
        if extra:
            k["rot"] = add(k["rot"], extra)
        return k

    # Wave (right hand), 2 s: raise, three waves of the forearm, lower.
    # The upper arm is raised to the side and turned out (Y -80) so the elbow bends upward; the wave
    # is the elbow opening and closing, which swings the forearm side to side, the hand trailing.
    up = {"leftShoulder": (8, 0, 0), "leftUpperArm": (22, -80, 78), "leftLowerArm": (48, 0, 0), "leftHand": (0, 0, 0)}
    keys = [(0, start), (10, with_arm(start, "right", up, "open", {"head": (0, -6, -4)}))]
    for i, f in enumerate(range(14, 46, 8)):
        w = dict(up)
        w["leftLowerArm"] = (28 if i % 2 == 0 else 58, 0, 0)
        w["leftHand"] = (0, 0, -14 if i % 2 == 0 else 14)
        keys.append((f, with_arm(start, "right", w, "open", {"head": (0, -6, -4), "hips": (0, 2, 0)})))
    keys += [(50, with_arm(start, "right", up, "open")), (60, start)]
    out.append(Keyed(rig, "wave", 60, False, keys, base=IDLE_BASE, lag=lag, priority=3))

    # Point (right hand) forward, 1.5 s.
    pt = {"leftShoulder": (2, 0, -6), "leftUpperArm": (78, -10, 14), "leftLowerArm": (6, 0, 0), "leftHand": (0, 0, 0)}
    look = {"head": (4, -10, 0), "neck": (2, -6, 0), "upperChest": (0, -6, 0), "chest": (2, -4, 0)}
    # The arm shoots out a little past the target (frame 7), settles (12) and drifts while held; the
    # chest leans into it.
    over = dict(pt, leftUpperArm=(86, -10, 16), leftLowerArm=(0, 0, 0))
    held = dict(pt, leftUpperArm=(75, -10, 13), leftLowerArm=(8, 0, 0))
    lean = dict(look, spine=(3, 0, 0), chest=(4, -4, 0))
    keys = [(0, start), (7, with_arm(start, "right", over, "point", lean)),
            (12, with_arm(start, "right", pt, "point", lean)),
            (32, with_arm(start, "right", held, "point", look)), (45, start)]
    out.append(Keyed(rig, "point", 45, False, keys, base=IDLE_BASE, lag=lag, priority=3))

    # Cheer, 1.6 s: a small dip, both fists thrown up with a hop onto the toes, two pumps, down.
    def cheer(arm_x, arm_z, elbow, hz, toes, shape="fist", lean=0.0):
        k = {"rot": dict(start["rot"]), "hips": (0.0, 0.0, hz),
             "feet": {"left": (0.160, ANKLE_L.y + 0.012 + (0.05 if toes else 0), ANKLE_L.z + (0.07 if toes else 0),
                               -30 if toes else 0, 12)}}
        arm = {"leftShoulder": (10 if max(arm_x, arm_z) > 90 else 0, 0, 0), "leftUpperArm": (arm_x, 0, arm_z),
               "leftLowerArm": (elbow, 0, 0), "leftHand": (0, 0, 0)}
        k["rot"].update(arm)
        k["rot"].update(mirror_rot(arm))
        k["rot"].update(hands(shape))
        k["rot"].update({"hips": (lean, 0, 0), "spine": (lean * 0.5, 0, 0), "head": (-lean - 6 if max(arm_x, arm_z) > 90 else 0, 0, 0)})
        return k

    # Fists go up in a V (raised to the sides, so they come down the same way, never forward).
    keys = [(0, start), (6, cheer(-10, -4, 40, -0.07, False, "fist", 10)), (12, cheer(4, 140, 20, 0.04, True)),
            (18, cheer(6, 122, 64, 0.0, True)), (24, cheer(4, 142, 18, 0.04, True)), (30, cheer(6, 122, 62, 0.0, True)),
            (38, cheer(2, 44, 30, -0.03, False, "relaxed")), (48, start)]
    out.append(Keyed(rig, "cheer", 48, False, keys, lag=lag, priority=3))

    # Dance, 2 s loop at 120 bpm (15 frames a beat): a two-step with a bounce on every beat, arms
    # pumping, head bobbing.
    def dance(beat, dz=0.0, up=False):
        side = 1 if beat in (0, 1) else -1          # weight goes left on beats 1-2, right on 3-4
        out_step = beat in (0, 2)
        ff = stand_feet(10, 0.15)["left"]
        tap = (ff[0] + 0.10, ff[1] - 0.02, ff[2] + 0.02, 18, 20)
        if side == 1:
            feet = {"left": ff, "right": tap if out_step else ff}
        else:
            feet = {"left": tap if out_step else ff, "right": ff}
        r = {"hips": (4, 10 * side, 4 * side), "spine": (2, -4 * side, -2 * side), "chest": (2, -6 * side, -2 * side),
             "neck": (-2, 0, 3 * side), "head": (4, 4 * side, 4 * side)}
        # Arms pump on every beat: the front fist punches up on the off-beat, the back elbow drives back.
        armf = {"leftUpperArm": (78, 0, 14) if up else (50, 0, 8), "leftLowerArm": (62 if up else 100, 0, 0),
                "leftHand": (0, 0, 0)}
        armb = {"leftUpperArm": (-24 if up else -10, 0, 14), "leftLowerArm": (90 if up else 80, 0, 0),
                "leftHand": (0, 0, 0)}
        r.update(armf if side == 1 else armb)
        r.update(mirror_rot(armb if side == 1 else armf))
        r.update(hands("fist"))
        r["head"] = (r["head"][0] + (-4 if up else 6), r["head"][1], r["head"][2])
        return {"rot": r, "hips": (0.045 * side, 0, -0.06 + dz), "feet": feet}

    keys = []
    for b in range(4):
        keys.append((b * 15, dance(b)))
        keys.append((b * 15 + 7, dance(b, 0.045, up=True)))
    out.append(Keyed(rig, "dance", 60, True, keys, lag=lag, priority=3, feet_mirror=False,
                     meta={"bpm": 120, "frames_per_beat": 15}))
    return out


def concept_poses(rig):
    """The concept sheet's three poses, as one-frame clips."""
    out = []

    def still(name, spec, export=True):
        out.append(Keyed(rig, name, 0, False, [(0, spec)], export=export, priority=0, feet_mirror=False))

    # Hand on hip: wide stance, weight on the figure's left leg, right fist on the right hip with the
    # elbow out, left arm hanging, head tilted.
    rot = {"hips": (0, 6, 5), "spine": (0, -2, -3), "chest": (-2, -4, -3), "upperChest": (-2, 0, 0),
           "neck": (0, 0, 2), "head": (2, 8, 6)}
    rot.update(side_rot({"leftUpperArm": (6, 0, 14), "leftLowerArm": (16, 0, 0), "leftHand": (8, -16, 6)}, "left"))
    rot.update(hand("loose", "left"))
    rot.update(hand("fist", "right"))
    rot.update({"rightShoulder": (4, 0, 0)})
    # A wide, confident A-stance: the feet about 0.75 m apart, legs straight, hips lowered to reach.
    still("pose_hand_on_hip", {"rot": rot, "hips": (0.03, 0, -0.085),
                               "feet": {"left": (0.38, ANKLE_L.y - 0.01, ANKLE_L.z, 0, 18),
                                        "right": (0.37, ANKLE_L.y + 0.02, ANKLE_L.z, 0, 22)},
                               "hands": {"right": (0.150, 0.03, 0.80, 0.25, -0.45, -0.85, -0.85, 0.2, 0.0)},
                               "pole": {"elbow": (1.0, 0.25, 0.0)}})

    # Run: the sheet's dynamic run. Strong forward lean, the right knee driven up and forward (thigh
    # near horizontal, shin hanging), the left leg pushing off behind on the ball of the foot; arms
    # swing big against the legs (left fist forward at chin height, right arm back), fists out from
    # the body, shoulders counter-rotated against the hips.
    rot = {"hips": (24, 12, 2), "spine": (8, -6, 0), "chest": (6, -8, 0), "upperChest": (2, -6, 0),
           "neck": (-14, 2, 0), "head": (-14, 4, 0)}
    rot.update(side_rot({"leftUpperArm": (78, 0, -2), "leftLowerArm": (95, 0, 0), "leftHand": (-6, 0, 0),
                         "leftShoulder": (2, 0, -12)}, "left"))
    rot.update(side_rot({"leftUpperArm": (-72, 0, 14), "leftLowerArm": (40, 0, 0), "leftHand": (6, 0, 0),
                         "leftShoulder": (0, 0, 12)}, "right"))
    rot.update(hands("fist"))
    rot["leftToes"] = (36, 0, 0)
    still("pose_run", {"rot": rot, "hips": (0, 0, -0.04),
                       "feet": {"left": (0.10, ANKLE_L.y + 0.30, 0.15, -46, 2),
                                "right": (0.11, -0.28, 0.34, -36, 4)}})

    # Sitting back on the hands: on the ground, leaning back on straight arms, the right leg stretched
    # long, the left knee bent up with its foot crossing in toward the right leg, head to the viewer.
    rot = {"hips": (-36, 8, 0), "spine": (4, 2, 0), "chest": (4, 2, 0), "upperChest": (2, 0, 0),
           "neck": (6, 8, 2), "head": (8, 12, 10)}
    rot.update(hands("flat"))
    hl = (0.23, 0.42, 0.04, 0.05, -0.85, -0.5, 0.0, 0.0, -1.0)
    hr = (0.25, 0.40, 0.04, 0.05, -0.85, -0.5, 0.0, 0.0, -1.0)
    still("pose_sit_back", {"rot": rot, "hips": (0, 0.10, -0.62),
                            "feet": {"left": (-0.02, -0.34, ANKLE_L.z, 0, -10),
                                     "right": (0.17, -0.70, 0.10, 18, 20, 10)},
                            "hands": {"left": hl, "right": hr}, "pole": {"elbow": (0.6, 1.0, 0.0)}})
    return out


def test_clips(rig):
    """Still poses for the review sheets (not exported): the hands panel and deformation tests."""
    out = []

    def still(name, spec):
        out.append(Keyed(rig, name, 0, False, [(0, spec)], export=False, priority=0, feet_mirror=False))

    still("test_rest", {"rot": {}})
    still("test_display", {"rot": hands("relaxed")})
    still("test_hand_fist", {"rot": hands("fist")})
    still("test_hand_open", {"rot": hands("open")})
    F = stand_feet(10, 0.17)["left"]
    # Deep knee bend: stand on the right leg, left thigh up, knee folded.
    rot = merge(arms_down(), hands("relaxed"), {"leftUpperLeg": (95, 0, 4), "leftLowerLeg": (-130, 0, 0),
                                                "leftFoot": (-15, 0, 0), "leftUpperArm": (4, 0, 24)})
    still("test_knee_bend", {"rot": rot, "hips": (0, 0, -0.01), "feet": {"right": F}})
    rot = merge(arms_down(), hands("open"), {"leftShoulder": (24, 0, 0), "leftUpperArm": (0, 0, 150),
                                              "leftLowerArm": (10, 0, 0)})
    still("test_arm_overhead", {"rot": rot, "feet": {"left": F, "right": F}})
    rot = merge(arms_down(), hands("relaxed"), {"leftShoulder": (0, 0, -10), "leftUpperArm": (108, 0, 40),
                                                "leftLowerArm": (22, 0, 0)})
    still("test_arm_across", {"rot": rot, "feet": {"left": F, "right": F}})
    rot = merge(arms_down(), hands("open"), {"leftUpperArm": (60, 0, 0), "leftLowerArm": (40, 0, 0),
                                              "leftHand": (50, 85, 0), "rightUpperArm": (60, 0, 0),
                                              "rightLowerArm": (40, 0, 0), "rightHand": (-55, 0, 0)})
    still("test_wrist_twist", {"rot": rot, "feet": {"left": F, "right": F}})
    # Crouch: feet wide and turned out, knees open over the toes so the torso sits between the thighs
    # (a clean silhouette instead of thighs pressed into the belly), forearms resting on the knees.
    rot = merge(hands("relaxed"), {"hips": (16, 0, 0), "spine": (8, 0, 0), "chest": (4, 0, 0), "neck": (-12, 0, 0),
                                   "head": (-10, 0, 0)},
                {"leftUpperArm": (52, 0, 4), "leftLowerArm": (52, 0, 0), "rightUpperArm": (52, 0, -4),
                 "rightLowerArm": (52, 0, 0)})
    cf = (0.24, ANKLE_L.y + 0.02, ANKLE_L.z, 0, 30)
    still("test_crouch", {"rot": rot, "hips": (0, 0.08, -0.40), "feet": {"left": cf, "right": cf},
                          "pole": {"knee": (38,)}})
    rot = merge(hands("open"), {"leftUpperArm": (0, 0, 60), "rightUpperArm": (0, 0, -60)})
    still("test_splits", {"rot": rot, "hips": (0, 0.0, -0.34),
                          "feet": {"left": (0.09, -0.52, 0.11, 30, 0), "right": (0.09, 0.50, 0.07, -50, 0)}})
    return out


def ual_clips(rig):
    import retarget_clips as RC
    src = RC.Source()
    R1 = RC.Retargeter(rig, src, "ual1")
    R2 = RC.Retargeter(rig, src, "ual2")
    two = 2 * math.pi
    head = RC.level_head(0.6)
    arm_lag = {"*LowerArm": 1.0, "*Hand": 2.0, "*Proximal": 2.0, "*Intermediate": 2.5, "*Distal": 3.0}
    out = []

    # ---- idle: the source idle, slowed to 3 s, with a breath and a slow weight shift added.
    def breathe(ctx):
        b = math.sin(two * ctx.u * 2)                # two breaths per loop
        ctx.rotate_local("upperChest", 0, -1.6 * b)  # chest rises (extends) on the in-breath
        for s in ("left", "right"):
            ctx.rotate_local(s + "Shoulder", 2, 1.2 * b * (1 if s == "left" else -1))
    toe_out = RC.toe_out(10)
    idle = RC.RetargetClip(rig, R1, "idle", "Idle_Loop", 90, True, spread=(1.0, 0.3),
                           layers=[breathe, RC.abduct(5), head, toe_out,
                                   RC.move_hips(lambda c: (0.008 * math.sin(two * c.u), 0, 0))],
                           lag={"head": 3, "neck": 2, **arm_lag})
    out.append(idle)

    # ---- gaits: procedural legs (exact contacts, same roll-in), source upper body.
    out.append(RetargetLoco(rig, "walk_slow", LOCOMOTION["walk_slow"], R1, "Walk_Loop",
                            layers=[RC.abduct(6), head], lag=arm_lag, stabilize={"head": 0.5}))
    out.append(RetargetLoco(rig, "walk", LOCOMOTION["walk"], R1, "Jog_Fwd_Loop",
                            layers=[RC.abduct(12), head], lag=arm_lag, stabilize={"head": 0.5},
                            exaggerate={"spine": 0.6, "chest": 0.6, "upperChest": 0.6},
                            hands={"left": ("loose", 0.6), "right": ("loose", 0.6)}))
    out.append(RetargetLoco(rig, "run", LOCOMOTION["run"], R1, "Sprint_Loop",
                            layers=[RC.turn("spine", (1, 0, 0), -8), RC.abduct(12), head], lag=arm_lag,
                            stabilize={"head": 0.5}, hands={"left": ("fist", 0.8), "right": ("fist", 0.8)}))

    # ---- jump: anticipation (0-4: stand to crouch), push-off (4-6), launch on frame 6, then the
    # source's take-off and tuck at 1.8x speed. The runtime starts at frame 6 and hands over to
    # the fall loop at the apex.
    def jump_time(f):
        if f <= 4:
            return 0.0
        if f <= 6:
            return f - 4
        return 2 + (f - 6) * 1.8
    # Arms (directions for the left arm: +x out, -y forward, +z up; the right is mirrored): back
    # in the crouch, then down, forward and UP through the launch (frame 6) with a little
    # overshoot (9), opening up and OUT at the apex (about 38 degrees from vertical, elbows soft,
    # fists outside the head's silhouette from front and back), then settling toward the fall
    # loop's arms (out to the sides) so jump -> fall blends cleanly.
    jump_arm = RC.aim("leftUpperArm", [(4, (0.22, 0.75, -0.62)), (5.5, (0.22, -0.05, -0.97)),
                                       (7.5, (0.45, -0.88, 0.10)), (9.5, (0.58, -0.50, 0.64)),
                                       (12.5, (0.62, -0.12, 0.78)), (16, (0.80, -0.10, 0.59))],
                      weight=[(2, 0.0), (4, 1.0)])
    jump_fore = RC.aim("leftLowerArm", [(4, (0.15, 0.45, -0.88)), (5.5, (0.15, -0.40, -0.90)),
                                        (7.5, (0.40, -0.80, 0.45)), (9.5, (0.48, -0.40, 0.78)),
                                        (12.5, (0.45, -0.20, 0.87)), (16, (0.70, -0.30, 0.65))],
                       weight=[(2, 0.0), (4, 1.0)])
    jump_chest = RC.lean("chest", [(0, (0, 0, 0)), (4, (8, 0, 0)), (9, (-7, 0, 0)), (16, (-3, 0, 0))])
    jr = RC.RetargetClip(rig, R1, "jump_src", "Jump_Start", 16, False, timemap=jump_time, feet="lock", spread=(1.0, 0.3),
                         layers=[jump_chest, jump_arm, jump_fore, RC.level_head(0.4), toe_out],
                         lag={"head": 2, "neck": 1, "*LowerArm": 1, "*Hand": 1.5},
                         hands={"left": ("loose", 0.9), "right": ("loose", 0.9)})
    stand = idle

    def jump(f):
        if f < 4:
            return RC.blend_pose(stand.sample(0), jr.sample(4), smooth(f / 4))
        return jr.sample(f)
    out.append(RC.FnClip("jump", 16, False, jump, meta={
        "launch_frame": 6, "source": "Quaternius UAL 1 Jump_Start (+ code anticipation 0-4)",
        "note": "frames 0-5 are the anticipation; apply the jump impulse on frame 6"}))

    # ---- fall: the source air loop, 2.5 s, legs FK.
    # Slow paddling of the arms and pedalling of the legs on top, so the fall never freezes.
    def flail(ctx):
        a = two * ctx.u * 3                     # 3 strokes per 2.5 s loop
        for s, sg in (("left", 1), ("right", -1)):
            ctx.rotate_world(s + "UpperArm", (1, 0, 0), 16 * math.sin(a + (0 if s == "left" else 1.3)))
            ctx.rotate_world(s + "LowerArm", (1, 0, 0), 12 * math.sin(a - 0.8 + (0 if s == "left" else 1.3)))
            ctx.rotate_world(s + "UpperLeg", (0, -sg, 0), 7)     # legs apart: no crossing shins
            ctx.rotate_world(s + "UpperLeg", (1, 0, 0), 10 * sg * math.sin(a * 2 / 3))
            ctx.rotate_world(s + "LowerLeg", (1, 0, 0), -9 * sg * math.sin(a * 2 / 3 - 0.9))
    relaxed = {"left": ("relaxed", 0.9), "right": ("relaxed", 0.9)}
    out.append(RC.RetargetClip(rig, R1, "fall", "Jump_Loop", 75, True, feet="fk", hands=relaxed,
                               layers=[RC.abduct(28), flail, RC.level_head(0.4)], lag={"head": 3, **arm_lag}))

    # ---- land (soft) and land_hard: the source landing; soft is shallower and quicker.
    def soften(k):
        def layer(ctx):
            rest = ctx.clip.R.tgt_head["hips"]
            ctx.hips = Vector((ctx.hips.x, ctx.hips.y, rest.z + (ctx.hips.z - rest.z) * k))
        return layer
    # Squash fast (touchdown to the deepest point in 3 frames instead of 6), recover slower.
    soft_time = RC.piecewise([(0, 0), (3, 6), (16, 28)])
    # The last frames ease into idle's first pose, so the hand-over to idle does not pop.
    def settle(clip, start):
        def fn(f):
            w = smooth((f - start) / (clip.length - start))
            return RC.blend_pose(clip.sample(f), idle.sample(0), w) if w > 0 else clip.sample(f)
        return fn
    land = RC.RetargetClip(rig, R1, "land", "Jump_Land", 16, False, timemap=soft_time, spread=(1.0, 0.3),
                               layers=[soften(0.6), RC.abduct(6), RC.level_head(0.5), toe_out],
                               exaggerate={"spine": 0.6, "chest": 0.6, "upperChest": 0.6},
                               lag={"head": 2, **arm_lag}, hands=relaxed)
    out.append(RC.FnClip("land", 16, False, settle(land, 8), meta=land.meta))
    hard_time = RC.piecewise([(0, 0), (3, 5.5), (8, 8), (28, 34)])
    hard = RC.RetargetClip(rig, R1, "land_hard", "Jump_Land", 28, False, timemap=hard_time, spread=(1.0, 0.3),
                           layers=[RC.abduct(6), RC.level_head(0.3), toe_out], lag={"head": 2, **arm_lag},
                           hands=relaxed)
    out.append(RC.FnClip("land_hard", 28, False, settle(hard, 16), meta=hard.meta))

    # ---- swim and swim idle, legs FK.
    out.append(RC.RetargetClip(rig, R1, "swim", "Swim_Fwd_Loop", 40, True, feet="fk", priority=2,
                               lag={"head": 2, **arm_lag}))
    out.append(RC.RetargetClip(rig, R1, "swim_idle", "Swim_Idle_Loop", 100, True, feet="fk", priority=2,
                               lag={"head": 3, **arm_lag}, hands=relaxed))

    # ---- sit, dance, idle_b.
    out.append(RC.RetargetClip(rig, R1, "sit", "Sitting_Idle_Loop", 60, True, priority=2,
                               layers=[RC.level_head(0.3)], lag={"head": 3, **arm_lag}, hands=relaxed))
    bounce = RC.move_hips(lambda c: (0, 0, -0.03 * (0.5 - 0.5 * math.cos(two * 2 * c.u))))
    out.append(RC.RetargetClip(rig, R1, "dance", "Dance_Loop", 30, True, priority=3,
                               layers=[RC.abduct(8), bounce, toe_out],
                               exaggerate={"*UpperArm": 1.6, "*LowerArm": 1.4, "spine": 1.6, "chest": 1.6,
                                           "upperChest": 1.4, "hips": 1.5, "head": 1.5},
                               lag={"head": 2, **arm_lag}, meta={"bpm": 120, "frames_per_beat": 15}))
    out.append(RC.RetargetClip(rig, R2, "idle_b", "Idle_FoldArms_Loop", 90, True, priority=2,
                               spread=(1.0, 0.3), layers=[head, toe_out], lag={"head": 3, "neck": 2},
                               hands={"left": ("relaxed", 0.5), "right": ("relaxed", 0.5)}))

    # ---- wave (right hand), 2 s, over the idle. The arm rises out to the side on an arc, the
    # forearm swings from upright-and-out toward upright (the hand stays clear of the big head),
    # the wrist trails each swing, the chest counters and the weight goes onto the left leg.
    idle_time = idle.timemap
    on = [(0, 0.0), (8, 1.0), (50, 1.0), (59, 0.0)]
    swing_a, swing_b = (-0.50, -0.20, 0.84), (-0.10, -0.22, 0.97)
    fore = [(0, (-0.15, -0.10, -0.98)), (5, (-0.85, -0.30, 0.42)), (11, swing_a)]
    for i, f in enumerate(range(16, 46, 5)):
        fore.append((f, swing_b if i % 2 == 0 else swing_a))
    fore += [(50, (-0.35, -0.25, 0.90)), (55, (-0.60, -0.20, -0.77)), (60, (-0.15, -0.10, -0.98))]
    wrist = [(0, 0)] + [(f, -16 if i % 2 == 0 else 16) for i, f in enumerate(range(11, 47, 5))] + [(50, 0)]
    wave_layers = [
        RC.hips_keys([(0, (0, 0, 0)), (10, (0.018, 0, -0.008)), (50, (0.018, 0, -0.008)), (60, (0, 0, 0))]),
        RC.lean("chest", [(0, (0, 0, 0)), (10, (0, 3, 0))]
                + [(f, (0, 3 + (1.5 if i % 2 == 0 else -1.5), 0)) for i, f in enumerate(range(16, 46, 5))]
                + [(50, (0, 2, 0)), (60, (0, 0, 0))]),
        RC.aim("rightUpperArm", [(0, (-0.20, 0.0, -0.98)), (5, (-0.92, -0.30, 0.05)), (10, (-0.92, -0.20, 0.36)),
                                 (14, (-0.90, -0.24, 0.30)), (50, (-0.90, -0.24, 0.30)),
                                 (55, (-0.80, -0.12, -0.58)), (60, (-0.20, 0.0, -0.98))],
               weight=on, mirror=False),
        RC.aim("rightLowerArm", fore, weight=on, mirror=False),
        RC.spin("rightHand", 2, wrist, mirror=False),
        RC.lean("head", [(0, (0, 0, 0)), (12, (-3, -5, -10)), (48, (-3, -5, -10)), (60, (0, 0, 0))]),
        head, toe_out]

    def wave_hands(f):
        w = RC.keyed([(0, 0.0), (6, 0.9), (50, 0.9), (58, 0.0)], f, lambda a, b, t: a + (b - a) * t)
        return {"right": ("open", w) if w > 0.05 else ("relaxed", 0.6), "left": ("relaxed", 0.6)}
    out.append(RC.RetargetClip(rig, R1, "wave", "Idle_Loop", 60, False, priority=3, timemap=idle_time,
                               spread=(1.0, 0.3), layers=wave_layers, hands=wave_hands,
                               lag={"head": 3, "neck": 2, "rightLowerArm": 1, "rightHand": 2.5, "*Proximal": 2,
                                    "*Intermediate": 2.5, "*Distal": 3},
                               meta={"source": "Quaternius UAL 1 Idle_Loop + keyed wave arm"}))

    # ---- cheer, 1.6 s, over the idle: anticipation (knees dip, chest curls, fists pulled back to
    # the hips), then both fists thrown up in a wide V with the chest opening and the head looking
    # up, two pumps with the knees, and back down. Head and forearms trail (lag).
    cheer_on = [(0, 0.0), (4, 1.0), (38, 1.0), (47, 0.0)]
    up_v, up_lo = (0.62, -0.15, 0.77), (0.50, -0.10, 0.86)
    pump, pump_lo = (0.86, -0.20, 0.34), (0.48, -0.22, 0.85)
    cheer_layers = [
        RC.hips_keys([(0, (0, 0, 0)), (7, (0, 0.012, -0.065)), (11, (0.006, 0, 0.02)), (14, (0.01, 0, 0)),
                      (18, (0.016, 0, -0.03)), (21, (0.016, 0, 0.01)), (25, (0.016, 0, -0.03)),
                      (28, (0.012, 0, 0.005)), (36, (0.006, 0, -0.012)), (48, (0, 0, 0))]),
        RC.lean("chest", [(0, (0, 0, 0)), (7, (11, 0, 0)), (11, (-9, 0, 0)), (18, (-3, 2, 0)), (21, (-7, 0, 0)),
                          (25, (-3, -2, 0)), (28, (-6, 0, 0)), (38, (0, 0, 0))]),
        RC.aim("leftUpperArm", [(0, (0.22, 0.0, -0.97)), (7, (0.22, 0.45, -0.86)), (9, (0.55, -0.35, -0.10)),
                                (11, up_v), (14, (0.58, -0.10, 0.80)), (18, pump), (21, up_v), (25, pump),
                                (28, up_v), (34, (0.62, -0.25, 0.55)), (41, (0.25, 0.0, -0.97))],
               weight=cheer_on),
        RC.aim("leftLowerArm", [(0, (0.15, -0.10, -0.98)), (7, (0.10, -0.75, -0.65)), (9, (0.40, -0.70, 0.30)),
                                (11, up_lo), (14, (0.45, -0.12, 0.88)), (18, pump_lo), (21, up_lo), (25, pump_lo),
                                (28, up_lo), (34, (0.30, -0.40, 0.86)), (41, (0.15, -0.10, -0.98))],
               weight=cheer_on),
        RC.lean("head", [(0, (0, 0, 0)), (7, (10, 0, 0)), (12, (-14, 0, 0)), (18, (-8, 3, 0)), (21, (-12, 0, 0)),
                         (25, (-8, -3, 0)), (28, (-10, 0, 0)), (40, (0, 0, 0))]),
        toe_out]

    def cheer_hands(f):
        w = RC.keyed([(0, 0.0), (5, 1.0), (36, 1.0), (44, 0.0)], f, lambda a, b, t: a + (b - a) * t)
        return {s: ("fist", w) if w > 0.05 else ("relaxed", 0.6) for s in ("left", "right")}
    out.append(RC.RetargetClip(rig, R1, "cheer", "Idle_Loop", 48, False, priority=3, timemap=idle_time,
                               spread=(1.0, 0.3), layers=cheer_layers, hands=cheer_hands,
                               lag={"head": 3, "neck": 2, "*LowerArm": 1.5, "*Hand": 2},
                               meta={"source": "Quaternius UAL 1 Idle_Loop + keyed cheer"}))
    return out


STANDING_UPPER = ("spine", "chest", "upperChest", "neck", "head")


def on_idle_legs(clip, idle):
    """A standing emote's upper body over the retargeted idle's hips, legs and feet, so the
    stance matches the idle it starts from and returns to (procedural emotes stand stiffer)."""
    import retarget_clips as RC

    def fn(f):
        base = idle.sample(f % idle.length)
        top = clip.sample(f)
        rot = dict(base["rot"])
        for b, r in top["rot"].items():
            if b in STANDING_UPPER or "Arm" in b or "Hand" in b or "Shoulder" in b or "Proximal" in b                     or "Intermediate" in b or "Distal" in b or "Metacarpal" in b:
                rot[b] = r
        return {**base, "rot": rot}
    return RC.FnClip(clip.name, clip.length, clip.loop, fn, priority=getattr(clip, "priority", 1),
                     meta={**getattr(clip, "meta", {}), "source": "procedural upper body over Quaternius UAL 1 Idle_Loop legs"})


def all_clips(arm):
    rig = Rig(arm)
    clips = ual_clips(rig)
    done = {c.name for c in clips}
    proc = idle_clips(rig) + jump_clips(rig) + p2_clips(rig) + emote_clips(rig)
    idle = next(c for c in clips if c.name == "idle")
    clips += [on_idle_legs(c, idle) if c.name == "point" else c
              for c in proc if c.name not in done]
    clips += concept_poses(rig) + test_clips(rig)
    return clips
