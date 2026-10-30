"""Build Metallic's default avatar (Phase A: mesh only, no rig) from nothing.

Run headless:
    blender -b --factory-startup --python art/avatar/build_avatar.py -- [--glb PATH] [--blend PATH]

Deterministic: no randomness, every number is in this file. The figure stands on z = 0, faces -Y
(glTF +Z after export), and its left side is +X. Units are metres. 1 stud = 0.28 m.

Construction: each body part is a closed shell. Limbs, torso, head and fingers are lofts of
rounded-rectangle rings; the boots are bevelled convex hulls on a lofted sole. Shells overlap at the
joints (mannequin style) so the later rig can keep each part near-rigid. Normals are face-area
weighted so the big planes shade flat and the narrow corner strips carry the soft bevel.
"""
import argparse
import math
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

STUD = 0.28
HEIGHT = 1.54  # 5.5 studs

X = Vector((1, 0, 0))
Y = Vector((0, 1, 0))
Z = Vector((0, 0, 1))
FRONT = Vector((0, -1, 0))

GREY = (0.42, 0.42, 0.42, 1.0)  # linear; about sRGB #ABABAB
SLOTS = ["Head", "Torso", "Arms", "Legs", "Hands", "Feet"]


# ---------------------------------------------------------------- 2D profiles

def rrect(hw, v0, v1, r, seg, ridge=None):
    """Rounded rectangle, u in [-hw, hw], v in [v0, v1]. r = radius or (r at v0 side, r at v1 side).
    Returns 4 * (seg + 1) points, counter-clockwise, starting at the (+u, v0) corner. With ridge =
    (front, back) a centre vertex is added on the front and back edges, pushed out by those amounts,
    which splits each of those planes into two (the concept's centre line on chest and back)."""
    r0, r1 = (r, r) if isinstance(r, (int, float)) else r
    hv = (v1 - v0) / 2
    r0 = max(0.0015, min(r0, hw * 0.98, hv * 0.98))
    r1 = max(0.0015, min(r1, hw * 0.98, hv * 0.98))
    corners = [(hw - r0, v0 + r0, r0, -90), (hw - r1, v1 - r1, r1, 0),
               (-hw + r1, v1 - r1, r1, 90), (-hw + r0, v0 + r0, r0, 180)]
    pts = []
    for i, (cu, cv, rr, a0) in enumerate(corners):
        for k in range(seg + 1):
            a = math.radians(a0 + 90.0 * k / seg)
            pts.append((cu + rr * math.cos(a), cv + rr * math.sin(a)))
        if ridge is not None and i == 1:
            pts.append((0.0, v1 + ridge[1]))   # back centre (spine line)
    if ridge is not None:
        pts.append((0.0, v0 - ridge[0]))       # front centre (sternum line)
    return pts


def offset_poly(pts, d):
    """Offset a counter-clockwise polygon outward by d (negative = inset), mitred corners."""
    n = len(pts)
    out = []
    for i in range(n):
        p0, p1, p2 = Vector(pts[i - 1]), Vector(pts[i]), Vector(pts[(i + 1) % n])
        e1, e2 = (p1 - p0).normalized(), (p2 - p1).normalized()
        n1, n2 = Vector((e1.y, -e1.x)), Vector((e2.y, -e2.x))
        m = (n1 + n2) / (1 + n1.dot(n2))
        q = p1 + m * d
        out.append((q.x, q.y))
    return out


# ---------------------------------------------------------------- shells

class Shell:
    """One closed part: a bmesh plus bookkeeping for sharp rings."""

    def __init__(self, name, slot):
        self.name, self.slot = name, slot
        self.bm = bmesh.new()


def station(o, u, v, pts):
    return {"o": Vector(o), "u": Vector(u).normalized(), "v": Vector(v).normalized(), "pts": pts}


def loft(shell, stations, sharp=(), sharp_cols=()):
    """Join rings with quads and close both ends with flat caps. `sharp` lists ring indices whose
    edges are marked sharp, so a plane break stays crisp; `sharp_cols` does the same for the edges
    running along the loft through those ring positions (the centre line of the chest)."""
    bm = shell.bm
    rings = []
    for st in stations:
        ring = [bm.verts.new(st["o"] + st["u"] * pu + st["v"] * pv) for pu, pv in st["pts"]]
        rings.append(ring)
    n = len(rings[0])
    faces = []
    for a, b in zip(rings, rings[1:]):
        for i in range(n):
            faces.append(bm.faces.new((a[i], a[(i + 1) % n], b[(i + 1) % n], b[i])))
    faces.append(bm.faces.new(list(reversed(rings[0]))))
    faces.append(bm.faces.new(rings[-1]))
    bm.edges.ensure_lookup_table()
    for k in sharp:
        ring = rings[k]
        for i in range(n):
            e = bm.edges.get((ring[i], ring[(i + 1) % n]))
            if e:
                e.smooth = False
    for i in sharp_cols:
        for a, b in zip(rings, rings[1:]):
            e = bm.edges.get((a[i], b[i]))
            if e:
                e.smooth = False
    bmesh.ops.recalc_face_normals(bm, faces=faces)
    return rings


def vertical_stations(rows, seg, cx=0.0, cy=0.0, ridge=False):
    """rows: (z, hw, front, back, r[, ridge_front, ridge_back]) with front < 0 < back along Y."""
    sts = []
    for row in rows:
        z, hw, f, b, r = row[:5]
        rg = (tuple(row[5:7]) if len(row) > 5 else (0.0, 0.0)) if ridge else None
        sts.append(station((cx, cy, z), X, Y, rrect(hw, f, b, r, seg, rg)))
    return sts


def path_stations(points, specs, ref_u, seg):
    """Rings along a polyline. spec = (segment, t, hw, hd, r[, du, dv[, twist_deg]]). At a joint
    (t = 0 on an inner segment) the ring plane bisects the two segments, so the joint gets its own
    loop. twist turns a ring about the path, e.g. the forearm turning toward the hand."""
    P = [Vector(p) for p in points]
    D = [(P[i + 1] - P[i]).normalized() for i in range(len(P) - 1)]
    sts = []
    for sp in specs:
        s, t, hw, hd, r = sp[:5]
        du, dv = (sp[5], sp[6]) if len(sp) > 5 else (0.0, 0.0)
        tw = math.radians(sp[7]) if len(sp) > 7 else 0.0
        o = P[s].lerp(P[s + 1], t)
        d = D[s]
        if t == 0.0 and s > 0:
            d = (D[s - 1] + D[s]).normalized()
        elif t == 1.0 and s < len(D) - 1:
            d = (D[s] + D[s + 1]).normalized()
        u = (ref_u - d * ref_u.dot(d)).normalized()
        v = d.cross(u).normalized()
        u, v = u * math.cos(tw) + v * math.sin(tw), v * math.cos(tw) - u * math.sin(tw)
        sts.append(station(o + u * du + v * dv, u, v, rrect(hw, -hd, hd, r, seg)))
    return sts


def hull_shell(shell, points, bevel=0.005, segments=2):
    """Bevelled convex hull: planar faces with a soft chamfer on every edge."""
    bm = shell.bm
    vs = [bm.verts.new(p) for p in points]
    res = bmesh.ops.convex_hull(bm, input=vs)
    drop = list({g for g in res["geom_interior"] + res["geom_unused"] if isinstance(g, bmesh.types.BMVert)})
    bmesh.ops.delete(bm, geom=drop, context="VERTS")
    bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(1.0), verts=bm.verts[:], edges=bm.edges[:])
    bmesh.ops.bevel(bm, geom=bm.edges[:] + bm.verts[:], offset=bevel, offset_type="OFFSET",
                    segments=segments, profile=0.5, affect="EDGES", clamp_overlap=True)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])


# ---------------------------------------------------------------- body parts (left side = +X)

def build_head():
    sh = Shell("Head", "Head")
    # z, half width, front (-Y), back (+Y), corner radius. Widest in the upper half, the jaw narrows,
    # and the back of the skull is cut away under a big chamfer toward the neck (concept side view).
    rows = [
        (1.168, 0.082, -0.118, 0.030, 0.036),
        (1.175, 0.112, -0.132, 0.044, 0.046),
        (1.188, 0.134, -0.142, 0.058, 0.056),
        (1.207, 0.151, -0.148, 0.074, 0.062),
        (1.233, 0.164, -0.152, 0.094, 0.068),
        (1.266, 0.174, -0.154, 0.121, 0.072),
        (1.305, 0.180, -0.155, 0.145, 0.075),
        (1.355, 0.184, -0.155, 0.152, 0.078),
        (1.410, 0.184, -0.153, 0.150, 0.078),
        (1.448, 0.177, -0.148, 0.144, 0.076),
        (1.475, 0.165, -0.138, 0.134, 0.072),
        (1.498, 0.147, -0.122, 0.118, 0.064),
        (1.516, 0.122, -0.101, 0.097, 0.054),
        (1.530, 0.093, -0.076, 0.072, 0.040),
        (1.540, 0.056, -0.044, 0.040, 0.024),
    ]
    loft(sh, vertical_stations(rows, seg=3))
    return sh


def build_neck():
    sh = Shell("Neck", "Head")
    rows = [
        (1.060, 0.043, -0.032, 0.054, 0.014),
        (1.110, 0.040, -0.030, 0.051, 0.013),
        (1.160, 0.038, -0.028, 0.049, 0.012),
        (1.230, 0.037, -0.027, 0.048, 0.012),
    ]
    loft(sh, vertical_stations(rows, seg=2))
    return sh


def build_torso():
    sh = Shell("Torso", "Torso")
    # A tapered, faceted chest: a plane break under the chest, a centre line front and back, and a
    # steep trapezius slope from the neck to the shoulders. The upper back sits a little behind the
    # chest so the posture is upright.
    rows = [
        (0.800, 0.078, -0.082, 0.060, 0.022, 0.000, 0.000),
        (0.845, 0.080, -0.085, 0.062, 0.022, 0.002, 0.001),
        (0.890, 0.089, -0.087, 0.071, 0.024, 0.003, 0.002),
        (0.935, 0.106, -0.096, 0.083, 0.026, 0.005, 0.003),   # chest plane break
        (0.990, 0.121, -0.097, 0.094, 0.028, 0.006, 0.004),
        (1.035, 0.136, -0.087, 0.100, 0.030, 0.005, 0.004),
        (1.062, 0.138, -0.072, 0.086, 0.030, 0.003, 0.003),
        (1.086, 0.114, -0.056, 0.072, 0.026, 0.001, 0.001),
        (1.102, 0.080, -0.044, 0.063, 0.022, 0.000, 0.000),
        (1.114, 0.050, -0.040, 0.058, 0.018, 0.000, 0.000),   # meets the neck
    ]
    loft(sh, vertical_stations(rows, seg=2, ridge=True), sharp=(3,), sharp_cols=(6, 13))
    return sh


def build_pelvis():
    sh = Shell("Pelvis", "Torso")
    rows = [
        (0.634, 0.026, -0.036, 0.044, 0.010, 0.000, 0.000),
        (0.648, 0.052, -0.056, 0.072, 0.016, 0.000, 0.000),   # brief wedge toward the crotch
        (0.672, 0.080, -0.068, 0.086, 0.020, 0.002, 0.002),   # front tucked behind the thighs
        (0.700, 0.106, -0.079, 0.094, 0.024, 0.003, 0.003),
        (0.745, 0.120, -0.089, 0.088, 0.027, 0.003, 0.003),   # hip, widest
        (0.795, 0.104, -0.089, 0.070, 0.026, 0.002, 0.002),
        (0.845, 0.084, -0.087, 0.065, 0.022, 0.002, 0.001),   # waist line
        (0.890, 0.076, -0.080, 0.060, 0.022, 0.000, 0.000),
    ]
    loft(sh, vertical_stations(rows, seg=2, ridge=True), sharp=(6,), sharp_cols=(6, 13))
    return sh


SHOULDER = Vector((0.130, 0.004, 1.010))
WRIST = Vector((0.262, -0.040, 0.642))
ARM_DIR = (WRIST - SHOULDER).normalized()
ELBOW = SHOULDER.lerp(WRIST, 0.51)


def build_arm():
    sh = Shell("Arm.L", "Arms")
    pts = [SHOULDER - ARM_DIR * 0.050, SHOULDER, ELBOW, WRIST, WRIST + ARM_DIR * 0.028]
    specs = [
        (0, 0.0, 0.015, 0.016, 0.007),
        (0, 0.30, 0.031, 0.034, 0.011),
        (0, 0.65, 0.040, 0.043, 0.013),
        (1, 0.0, 0.044, 0.046, 0.014),    # deltoid, widest
        (1, 0.45, 0.041, 0.044, 0.013),
        (1, 0.93, 0.039, 0.042, 0.013),
        (2, 0.0, 0.041, 0.044, 0.013),    # elbow loop
        (2, 0.07, 0.050, 0.051, 0.015),   # forearm step
        (2, 0.45, 0.055, 0.054, 0.016),
        (2, 0.85, 0.050, 0.050, 0.015, 0.0, 0.0, 10),
        (3, 0.0, 0.044, 0.044, 0.014, 0.0, 0.0, 22),    # wrist loop, turning toward the hand
        (3, 1.0, 0.041, 0.041, 0.013, 0.0, 0.0, 30),
    ]
    loft(sh, path_stations(pts, specs, X, seg=2), sharp=(7,))
    return sh


HAND_SCALE = 1.12


def hand_frame():
    """Wrist frame: a = down the arm, b = toward the front, c = toward the body (palm normal).
    The hand is turned so the back of the hand faces out and a little forward."""
    a = ARM_DIR.copy()
    b = (FRONT - a * FRONT.dot(a)).normalized()
    c = a.cross(b).normalized()
    if c.x > 0:
        c = -c
    tw = math.radians(60)
    b2 = (b * math.cos(tw) + c * math.sin(tw)).normalized()
    c2 = (c * math.cos(tw) - b * math.sin(tw)).normalized()
    return a, b2, c2


def build_hand():
    """Relaxed mitten hand: palm block, four curled finger blocks with a joint loop each, and a
    three-block thumb lying over the curled fingers."""
    sh = Shell("Hand.L", "Hands")
    a, b, c = hand_frame()
    o = WRIST + a * 0.006
    v = a.cross(b).normalized()
    # Palm: rings along a; u = breadth (b), v = thickness (toward the palm side).
    # The palm starts inside the forearm's end so the wrist reads as one clean line.
    rows = [(0.008, 0.032, 0.022), (0.020, 0.044, 0.028), (0.036, 0.053, 0.031),
            (0.072, 0.059, 0.031), (0.096, 0.058, 0.029), (0.106, 0.053, 0.022)]
    sts = [station(o + a * t + c * 0.006, b, v, rrect(hb, -hc, hc, 0.012, 2)) for t, hb, hc in rows]
    loft(sh, sts, sharp=(2,))

    # Fingers, index at the front. Proximal and distal blocks curled toward the palm.
    knuckle = o + a * 0.092 + c * (0.006 - 0.029 + 0.0215)
    fingers = [(+0.0420, 0.93), (+0.0140, 1.0), (-0.0140, 0.95), (-0.0420, 0.82)]
    th1, th2 = math.radians(52), math.radians(70)
    for bi, sc in fingers:
        k = knuckle + b * bi
        L1, L2 = 0.060 * sc, 0.050 * sc
        d1 = (a * math.cos(th1) + c * math.sin(th1)).normalized()
        d2 = (a * math.cos(th1 + th2) + c * math.sin(th1 + th2)).normalized()
        p0 = k - a * 0.016
        p2 = k + d1 * L1
        p3 = p2 + d2 * L2
        specs = [
            (0, 0.0, 0.0128, 0.0190, 0.004),
            (1, 0.0, 0.0135, 0.0225, 0.0045),   # knuckle loop
            (1, 0.55, 0.0135, 0.0215, 0.0045),
            (2, 0.0, 0.0133, 0.0210, 0.0045),   # middle joint loop
            (2, 0.80, 0.0128, 0.0195, 0.0042),
            (2, 1.0, 0.0095, 0.0140, 0.0030),   # bevelled tip
        ]
        loft(sh, path_stations([p0, k, p2, p3], specs, b, seg=1))

    # Thumb: from the front edge of the palm, down, forward and in, over the curled fingers.
    base = o + a * 0.036 + b * 0.034 + c * 0.012
    t1 = (a * 0.50 + b * 0.75 + c * 0.32).normalized()
    t2 = (a * 0.80 + b * 0.38 + c * 0.42).normalized()
    t3 = (a * 0.82 + b * 0.05 + c * 0.55).normalized()
    q0 = base - t1 * 0.020
    q1 = base
    q2 = q1 + t1 * 0.042
    q3 = q2 + t2 * 0.036
    q4 = q3 + t3 * 0.030
    specs = [
        (0, 0.0, 0.017, 0.018, 0.005),
        (1, 0.0, 0.019, 0.021, 0.006),
        (1, 0.6, 0.0175, 0.020, 0.0055),
        (2, 0.0, 0.0168, 0.019, 0.0055),   # thumb joint loops
        (2, 0.6, 0.0160, 0.018, 0.005),
        (3, 0.0, 0.0155, 0.0172, 0.005),
        (3, 0.75, 0.0145, 0.0155, 0.0045),
        (3, 1.0, 0.0100, 0.0110, 0.0032),
    ]
    loft(sh, path_stations([q0, q1, q2, q3, q4], specs, c, seg=1))
    # The concept's hands are big mittens: scale the finished hand about the wrist.
    bmesh.ops.scale(sh.bm, vec=(HAND_SCALE,) * 3, space=Matrix.Translation(-WRIST), verts=sh.bm.verts[:])
    return sh


HIP = Vector((0.064, 0.000, 0.730))
KNEE = Vector((0.126, 0.012, 0.400))
ANKLE = Vector((0.178, 0.030, 0.105))


def build_leg():
    sh = Shell("Leg.L", "Legs")
    pts = [HIP, KNEE, ANKLE]
    # dv < 0 moves a ring toward the back (calf).
    specs = [
        (0, 0.0, 0.064, 0.074, 0.018),
        (0, 0.12, 0.068, 0.078, 0.019),
        (0, 0.28, 0.068, 0.077, 0.020),   # crotch level, thigh widest
        (0, 0.60, 0.060, 0.068, 0.018),
        (0, 0.93, 0.055, 0.062, 0.017),
        (1, 0.0, 0.057, 0.064, 0.017),    # knee loop
        (1, 0.07, 0.066, 0.069, 0.019, 0.0, 0.000),   # lower-leg step
        (1, 0.35, 0.071, 0.080, 0.020, 0.0, -0.010),   # calf
        (1, 0.72, 0.067, 0.075, 0.019, 0.0, -0.008),
        (1, 1.0, 0.063, 0.070, 0.018, 0.0, -0.006),
    ]
    sts = path_stations(pts, specs, X, seg=2)
    # Tilt the top two rings so the thigh rises higher on the outside: the hip reads as one plane
    # and the pelvis meets the thighs in the concept's V-shaped brief line.
    for st, tilt, lift in ((sts[0], 40, 0.010), (sts[1], 18, 0.0)):
        t = math.radians(tilt)
        up = (Z - st["v"] * Z.dot(st["v"])).normalized()
        st["u"] = (st["u"] * math.cos(t) + up * math.sin(t)).normalized()
        st["o"] = st["o"] + Vector((0.006, 0.0, lift))
    loft(sh, sts, sharp=(6,))
    return sh


def boot_footprint():
    """Sole outline in boot space (lx outward, ly forward), counter-clockwise from the heel."""
    pts = [(-0.054, -0.112), (0.058, -0.112), (0.080, -0.094), (0.084, -0.010), (0.100, 0.104),
           (0.092, 0.180), (0.066, 0.206), (-0.056, 0.206), (-0.080, 0.180), (-0.088, 0.104),
           (-0.074, -0.010), (-0.076, -0.094)]
    return [(x * BOOT_W, y) for x, y in pts]


# Tread pads under the sole: heel and forefoot, with the arch band between them.
BOOT_W = 1.16  # width scale of the sole and pads

HEEL_PAD = [(-0.044, -0.100), (0.048, -0.100), (0.066, -0.084), (0.070, -0.026), (-0.062, -0.026),
            (-0.066, -0.084)]
FORE_PAD = [(-0.066, 0.034), (0.076, 0.034), (0.088, 0.104), (0.079, 0.168), (0.053, 0.192),
            (-0.043, 0.192), (-0.067, 0.168), (-0.076, 0.104)]


def build_boot():
    """Heavy boot: tread pads, a thick sole slab with chamfered edges, and a wedge upper whose sides
    taper from the sole to the ankle and whose instep runs down to a bevelled toe cap."""
    sh = Shell("Boot.L", "Feet")
    cx, cy = ANKLE.x + 0.012, ANKLE.y

    def w(lx, ly, z):
        return Vector((cx + lx, cy - ly, z))

    for pad in ([(x * BOOT_W, y) for x, y in HEEL_PAD], [(x * BOOT_W, y) for x, y in FORE_PAD]):
        rows = [(0.000, -0.003), (0.003, 0.0), (0.010, 0.0)]
        loft(sh, [station((cx, cy, z), X, FRONT, offset_poly(pad, d)) for z, d in rows])
    fp = boot_footprint()
    rows = [(0.005, -0.005), (0.012, 0.0), (0.046, 0.0), (0.056, -0.008)]
    loft(sh, [station((cx, cy, z), X, FRONT, offset_poly(fp, d)) for z, d in rows])

    up = [w(lx, ly, 0.048) for lx, ly in fp]
    # The cuff follows the leg, which leans in: its centre at the cuff is about 2.4 cm inside cx.
    lc = -0.024
    up += [w(-0.066, -0.104, 0.125), w(0.066, -0.104, 0.125),
           w(lc - 0.058, -0.068, 0.155), w(lc + 0.058, -0.068, 0.155),
           w(lc - 0.059, 0.000, 0.184), w(lc + 0.059, 0.000, 0.184),
           w(lc - 0.057, 0.056, 0.174), w(lc + 0.057, 0.056, 0.174),
           w(-0.072, 0.132, 0.108), w(0.084, 0.132, 0.108),
           w(-0.072, 0.172, 0.092), w(0.084, 0.172, 0.092),
           w(-0.066, 0.200, 0.074), w(0.076, 0.200, 0.074)]
    up_shell = Shell("tmp", "Feet")
    hull_shell(up_shell, up, bevel=0.007, segments=2)
    # A loop across the ball of the foot for a later toes bone.
    bm = up_shell.bm
    bmesh.ops.bisect_plane(bm, geom=bm.verts[:] + bm.edges[:] + bm.faces[:],
                           plane_co=w(0, 0.110, 0), plane_no=FRONT)
    merge_into(sh.bm, bm)
    bm.free()
    return sh


def merge_into(dst, src):
    m = bpy.data.meshes.new("tmp_merge")
    src.to_mesh(m)
    dst.from_mesh(m)
    bpy.data.meshes.remove(m)


# ---------------------------------------------------------------- assembly

def shell_to_object(sh, mirror=False, name=None):
    bm = sh.bm.copy()
    if mirror:
        for v in bm.verts:
            v.co.x = -v.co.x
        bmesh.ops.reverse_faces(bm, faces=bm.faces[:])
    for f in bm.faces:
        f.smooth = True
    me = bpy.data.meshes.new(name or sh.name)
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name or sh.name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def make_materials():
    mats = {}
    for s in SLOTS:
        m = bpy.data.materials.new("M_" + s)
        bsdf = next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        bsdf.inputs["Base Color"].default_value = GREY
        bsdf.inputs["Roughness"].default_value = 0.65
        m.diffuse_color = GREY
        mats[s] = m
    return mats


def build(glb=None, blend=None):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    shells = [build_head(), build_neck(), build_torso(), build_pelvis()]
    paired = [build_arm(), build_hand(), build_leg(), build_boot()]
    objs = [(shell_to_object(s), s.slot) for s in shells]
    for s in paired:
        objs.append((shell_to_object(s, name=s.name), s.slot))
        objs.append((shell_to_object(s, mirror=True, name=s.name[:-2] + ".R"), s.slot))
    for s in shells + paired:
        s.bm.free()

    mats = make_materials()
    for ob, slot in objs:
        for s in SLOTS:
            ob.data.materials.append(mats[s])
        idx = SLOTS.index(slot)
        for p in ob.data.polygons:
            p.material_index = idx
        vg = ob.vertex_groups.new(name=ob.name)
        vg.add(list(range(len(ob.data.vertices))), 1.0, "REPLACE")

    # Join into one mesh; the part names live on as vertex groups.
    bpy.ops.object.select_all(action="DESELECT")
    for ob, _ in objs:
        ob.select_set(True)
    root = objs[0][0]
    bpy.context.view_layer.objects.active = root
    bpy.ops.object.join()
    root.name = "DefaultAvatar"
    root.data.name = "DefaultAvatar"

    # Face-area weighted normals: big planes flat, bevel strips soft, marked rings crisp.
    mod = root.modifiers.new("WeightedNormal", "WEIGHTED_NORMAL")
    mod.mode = "FACE_AREA"
    mod.weight = 80
    mod.keep_sharp = True
    bpy.ops.object.modifier_apply(modifier=mod.name)

    # One UV set, angle-based islands so each plane gets its own island for decals later.
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(60), island_margin=0.004, area_weight=0.0,
                             correct_aspect=True, scale_to_bounds=False)
    bpy.ops.object.mode_set(mode="OBJECT")
    root.data.uv_layers[0].name = "UVMap"

    tris = sum(len(p.vertices) - 2 for p in root.data.polygons)
    zs = [v.co.z for v in root.data.vertices]
    xs = [v.co.x for v in root.data.vertices]
    ys = [v.co.y for v in root.data.vertices]
    height = max(zs) - min(zs)
    print(f"[avatar] triangles={tris} vertices={len(root.data.vertices)}")
    print(f"[avatar] height={height:.4f} m = {height / STUD:.3f} studs; "
          f"width={max(xs) - min(xs):.3f} m depth={max(ys) - min(ys):.3f} m")

    if glb:
        os.makedirs(os.path.dirname(glb), exist_ok=True)
        bpy.ops.object.select_all(action="DESELECT")
        root.select_set(True)
        bpy.ops.export_scene.gltf(filepath=glb, export_format="GLB", use_selection=True,
                                  export_normals=True, export_materials="EXPORT",
                                  export_yup=True, export_apply=True)
        print(f"[avatar] glb={glb} bytes={os.path.getsize(glb)}")
    if blend:
        os.makedirs(os.path.dirname(blend), exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=blend, compress=True)
    return root, tris


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser()
    ap.add_argument("--glb", default=os.path.join(here, "out", "default_avatar_phaseA.glb"))
    ap.add_argument("--blend", default=None)
    ap.add_argument("--no-glb", action="store_true")
    a = ap.parse_args(argv)
    build(None if a.no_glb else a.glb, a.blend)


if __name__ == "__main__":
    main()
