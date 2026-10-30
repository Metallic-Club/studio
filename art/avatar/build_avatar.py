"""Build Metallic's default avatar mesh from nothing (Phase A model, Phase B fixes and LODs).

Run headless:
    blender -b --factory-startup --python art/avatar/build_avatar.py -- [--glb PATH] [--blend PATH]
        [--lod 0|1|2] [--head-seg N]

Deterministic: no randomness, every number is in this file. The figure stands on z = 0, faces -Y
(glTF +Z after export), and its left side is +X. Units are metres. 1 stud = 0.28 m.

Construction: each body part is a closed shell. Limbs, torso, head and fingers are lofts of
rounded-rectangle rings; the boots are bevelled convex hulls on a lofted sole. Shells overlap at the
joints (mannequin style) so the rig can keep each part rigid:
- ball joints (shoulder, hip) end in a dome centred on the pivot, so the part turns inside itself;
- hinge joints (elbow, knee) are two shells: the upper one ends in a half cylinder around the hinge
  axis, the lower one starts with a wider rim just outside that cylinder, so a deep bend never
  opens a gap (rigid weights, no blend needed);
- fingers and the thumb are single lofts with a joint ring at each knuckle (short blend in the rig).
Normals are face-area weighted so the big planes shade flat and the narrow corner strips carry the
soft bevel. `SKELETON` holds the joint positions the rig is built on (left side; the right side is
the mirror in X).
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

LOD = 0          # 0 = full, 1 = about half the triangles, 2 = about 15%
HEAD_SEG = 3     # unused since v3 (the head is an implicit surface, see HEAD_ROWS); kept so --head-seg still parses


def sg(n):
    """Corner segments for the current LOD."""
    return n if LOD == 0 else (max(1, n - 1) if LOD == 1 else 1)


def keep(items, lod1=None, lod2=None):
    """Pick rings for the current LOD: lod1/lod2 are index lists into items (None = all)."""
    pick = lod1 if LOD == 1 else (lod2 if LOD == 2 else None)
    return items if pick is None else [items[i] for i in pick]


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


def box_pts(hw, v0, v1):
    """Plain rectangle ring (LOD2)."""
    return [(hw, v0), (hw, v1), (-hw, v1), (-hw, v0)]


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
    """One closed part: a bmesh plus the vertex groups it belongs to."""

    def __init__(self, name, slot, groups=()):
        self.name, self.slot, self.groups = name, slot, tuple(groups)
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
        if k >= len(rings):
            continue
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


def axis_stations(o, d, ref_u, rows, seg):
    """Rings along one straight axis d from o. row = (s, hw, hd, r[, dv]) with dv a shift along v."""
    d = Vector(d).normalized()
    u = (ref_u - d * ref_u.dot(d)).normalized()
    v = d.cross(u).normalized()
    sts = []
    for row in rows:
        s, hw, hd, r = row[:4]
        dv = row[4] if len(row) > 4 else 0.0
        sts.append(station(Vector(o) + d * s + v * dv, u, v, rrect(hw, -hd, hd, r, seg)))
    return sts


def dome_rows(R_w, R_d, r, angles, sign=-1.0):
    """Rings that close a limb end as a dome centred on the joint pivot (s = 0): a ball joint turns
    inside itself. sign -1 puts the dome before the pivot (the top of the arm)."""
    rows = []
    for ang in angles:
        a = math.radians(ang)
        rows.append((sign * R_d * math.sin(a), max(0.004, R_w * math.cos(a)), max(0.004, R_d * math.cos(a)),
                     r * max(0.35, math.cos(a))))
    return rows


def hinge_rows(hw, R, r, angles):
    """Rings past a hinge pivot (s >= 0) that keep the lateral width and shrink the depth as a circle
    of radius R: the end becomes a half cylinder around the hinge axis (the ring's u axis)."""
    rows = []
    for ang in angles:
        a = math.radians(ang)
        rows.append((R * math.sin(a), hw * (1.0 if ang < 60 else 0.96), max(0.006, R * math.cos(a)),
                     min(r, R * math.cos(a) * 0.9)))
    return rows


def hull_shell(shell, points, bevel=0.005, segments=2):
    """Bevelled convex hull: planar faces with a soft chamfer on every edge."""
    bm = shell.bm
    vs = [bm.verts.new(p) for p in points]
    res = bmesh.ops.convex_hull(bm, input=vs)
    drop = list({g for g in res["geom_interior"] + res["geom_unused"] if isinstance(g, bmesh.types.BMVert)})
    bmesh.ops.delete(bm, geom=drop, context="VERTS")
    bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(1.0), verts=bm.verts[:], edges=bm.edges[:])
    if segments > 0:
        bmesh.ops.bevel(bm, geom=bm.edges[:] + bm.verts[:], offset=bevel, offset_type="OFFSET",
                        segments=segments, profile=0.5, affect="EDGES", clamp_overlap=True)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    if segments == 0:
        # No chamfer to carry the turn: keep the corners crisp, or the big caps darken the sides.
        for e in bm.edges:
            if len(e.link_faces) == 2 and e.calc_face_angle(0.0) > math.radians(35):
                e.smooth = False


# ---------------------------------------------------------------- skeleton (left side = +X)

# v3 joint positions, measured on the concept's front, side and back views (1 px = 2.47 mm at the
# sheet's figure height). The v2 values are in docs/agents/avatar-v3-model.md.
SHOULDER = Vector((0.122, 0.000, 1.046))
WRIST = Vector((0.270, -0.046, 0.626))
ARM_DIR = (WRIST - SHOULDER).normalized()
# The elbow sits 1.2 cm behind the straight line, so the rest pose has a visible soft bend (the
# concept's arms are not locked straight): the IK knows which way the elbow goes, and the hinge
# axis is well defined. The upper arm is the shorter segment, as on the sheet.
ELBOW = SHOULDER.lerp(WRIST, 0.465) + Vector((0.0, 0.012, 0.0))
UPPER_DIR = (ELBOW - SHOULDER).normalized()
LOWER_DIR = (WRIST - ELBOW).normalized()


def lateral(d):
    """Hinge axis of a limb segment along d that bends forward or back: perpendicular to d and to
    the front direction, pointing outward (+X)."""
    f = (FRONT - d * FRONT.dot(d)).normalized()
    ax = d.cross(f).normalized()
    return ax if ax.x > 0 else -ax


HIP = Vector((0.070, 0.000, 0.735))
KNEE = Vector((0.122, -0.004, 0.412))  # 1.3 cm ahead of the straight line: the knee's bend direction
ANKLE = Vector((0.175, 0.026, 0.112))
THIGH_DIR = (KNEE - HIP).normalized()
SHIN_DIR = (ANKLE - KNEE).normalized()

HAND_SCALE = 1.04   # v2: 1.24, a clump; the concept's hand is about 1.35 forearm-end widths across
HAND_TWIST = 40.0   # degrees: the back of the hand turns out and forward (the concept's front view)

# Boot frame: the boot is built around (cx, cy) with lx outward and ly forward.
BOOT_CX = ANKLE.x + 0.012
BOOT_CY = ANKLE.y
BALL_LY = 0.102     # the loop across the ball of the foot (toes bone pivot)
TOE_LY = 0.196
HEEL_LY = -0.102


def hand_frame():
    """Wrist frame: a = down the forearm, b = thumb side, c = palm normal (toward the palm)."""
    a = LOWER_DIR.copy()
    b = (FRONT - a * FRONT.dot(a)).normalized()
    c = a.cross(b).normalized()
    if c.x > 0:
        c = -c
    tw = math.radians(HAND_TWIST)
    b2 = (b * math.cos(tw) + c * math.sin(tw)).normalized()
    c2 = (c * math.cos(tw) - b * math.sin(tw)).normalized()
    return a, b2, c2


def H(s, w, n):
    """A point of the left hand in hand coordinates (s along a, w along b, n along c), scaled."""
    a, b, c = hand_frame()
    return WRIST + (a * s + b * w + c * n) * HAND_SCALE


# Hand layout in unscaled hand units.
PALM_N0 = 0.004          # palm centre offset toward the palm
KNUCKLE_S = 0.101        # finger base joints (MCP)
FINGER_N = -0.0028       # finger centre: the finger backs continue the back of the hand
FINGER_HW = 0.0138       # half width; spacing 0.029 leaves a 2 mm groove, so each finger block reads
FINGER_HD = 0.0198
FINGERS = {              # name: (w offset, length scale, width scale)
    "Index": (+0.0425, 0.94, 1.0),
    "Middle": (+0.0142, 1.00, 1.0),
    "Ring": (-0.0142, 0.95, 1.0),
    "Little": (-0.0420, 0.80, 0.94),
}
FINGER_L = (0.051, 0.035, 0.030)   # proximal, intermediate, distal of the middle finger
THUMB_L = (0.040, 0.034, 0.029)    # metacarpal, proximal, distal


def thumb_points():
    a, b, c = hand_frame()
    t1 = (a * 0.62 + b * 0.70 + c * 0.36).normalized()
    t2 = (a * 0.80 + b * 0.46 + c * 0.40).normalized()
    t3 = (a * 0.86 + b * 0.30 + c * 0.42).normalized()
    q1 = H(0.020, 0.036, 0.012)                 # CMC pivot, inside the palm's thumb edge
    q0 = q1 - t1 * 0.016 * HAND_SCALE
    q2 = q1 + t1 * THUMB_L[0] * HAND_SCALE
    q3 = q2 + t2 * THUMB_L[1] * HAND_SCALE
    q4 = q3 + t3 * THUMB_L[2] * HAND_SCALE
    return [q0, q1, q2, q3, q4]


def finger_points(name):
    w, ls, _ws = FINGERS[name]
    a, _b, _c = hand_frame()
    p0 = H(KNUCKLE_S, w, FINGER_N)
    L = [x * ls * HAND_SCALE for x in FINGER_L]
    p1 = p0 + a * L[0]
    p2 = p1 + a * L[1]
    p3 = p2 + a * L[2]
    return [p0, p1, p2, p3]


def skeleton():
    """Joint positions (left side) the rig is built on."""
    sk = {
        "SHOULDER": SHOULDER, "ELBOW": ELBOW, "WRIST": WRIST, "HAND_END": H(KNUCKLE_S, 0.0145, FINGER_N),
        "HIP": HIP, "KNEE": KNEE, "ANKLE": ANKLE,
        "BALL": Vector((BOOT_CX - 0.006, BOOT_CY - BALL_LY, 0.035)),
        "TOE": Vector((BOOT_CX - 0.006, BOOT_CY - TOE_LY + 0.004, 0.035)),
        "HEEL": Vector((BOOT_CX, BOOT_CY - HEEL_LY, 0.0)),
        "THUMB": thumb_points()[1:],
    }
    for f in FINGERS:
        sk[f.upper()] = finger_points(f)
    return sk


# ---------------------------------------------------------------- body parts

# Head profile, read off the concept's front and side views (metres; front = -Y). Each row is one
# horizontal section: z, half width, front, back, superellipse exponent. The face is a tall, nearly
# flat plane down to the chin; the jaw narrows in the lower third (front view); under the chin the
# head rises toward the neck, and the back of the skull turns under in a broad round chamfer.
HEAD_ROWS = [
    (1.5400, 0.000, -0.030, 0.020, 2.2),
    (1.5370, 0.062, -0.062, 0.044, 2.2),
    (1.5280, 0.097, -0.094, 0.068, 2.3),
    (1.5130, 0.125, -0.126, 0.093, 2.4),
    (1.4950, 0.146, -0.146, 0.112, 2.5),
    (1.4750, 0.161, -0.155, 0.128, 2.6),
    (1.4500, 0.173, -0.159, 0.140, 3.0),
    (1.4200, 0.181, -0.161, 0.147, 3.3),
    (1.3850, 0.185, -0.162, 0.149, 3.4),
    (1.3400, 0.186, -0.163, 0.148, 3.4),
    (1.3050, 0.185, -0.163, 0.143, 3.3),
    (1.2800, 0.181, -0.162, 0.132, 3.1),
    (1.2550, 0.174, -0.160, 0.112, 3.0),
    (1.2300, 0.166, -0.158, 0.086, 2.9),
    (1.2050, 0.150, -0.153, 0.068, 2.7),
    (1.1880, 0.131, -0.146, 0.046, 2.6),
    (1.1750, 0.108, -0.134, 0.020, 2.5),
    (1.1665, 0.084, -0.116, -0.010, 2.4),
    (1.1615, 0.056, -0.094, -0.046, 2.3),
    (1.1595, 0.000, -0.077, -0.060, 2.2),
]
# Seen from behind, the bottom of the head stays broad (the jaw corners): the lower rows widen
# toward the back by this factor (z, extra width at the back).
HEAD_JAW = [(1.300, 0.0), (1.250, 0.015), (1.210, 0.035), (1.188, 0.055), (1.175, 0.075), (1.160, 0.10)]
HEAD_CENTRE = Vector((0.0, -0.008, 1.350))
HEAD_GRID = {0: 10, 1: 5, 2: 3}      # cube-sphere cuts per face edge for each LOD


def _cubic(table, z, col):
    """Catmull-Rom through table[*][col] against table[*][0] (z ascending is not required)."""
    rows = sorted(table, key=lambda r: r[0])
    zs = [r[0] for r in rows]
    if z <= zs[0]:
        return rows[0][col]
    if z >= zs[-1]:
        return rows[-1][col]
    i = max(k for k in range(len(zs) - 1) if zs[k] <= z)
    p = [rows[max(0, min(len(rows) - 1, j))][col] for j in (i - 1, i, i + 1, i + 2)]
    q = [rows[max(0, min(len(rows) - 1, j))][0] for j in (i - 1, i, i + 1, i + 2)]
    t = (z - q[1]) / (q[2] - q[1])
    # non-uniform tangents (finite differences), then a Hermite segment
    m1 = (p[2] - p[0]) / (q[2] - q[0]) * (q[2] - q[1]) if q[2] != q[0] else 0.0
    m2 = (p[3] - p[1]) / (q[3] - q[1]) * (q[2] - q[1]) if q[3] != q[1] else 0.0
    # clamp the tangents (Fritsch-Carlson) so the profile never overshoots between rows
    d = p[2] - p[1]
    m1 = 0.0 if m1 * d <= 0 else max(-3 * abs(d), min(3 * abs(d), m1))
    m2 = 0.0 if m2 * d <= 0 else max(-3 * abs(d), min(3 * abs(d), m2))
    h00, h10 = 2 * t ** 3 - 3 * t ** 2 + 1, t ** 3 - 2 * t ** 2 + t
    h01, h11 = -2 * t ** 3 + 3 * t ** 2, t ** 3 - t ** 2
    return h00 * p[1] + h10 * m1 + h01 * p[2] + h11 * m2


def head_inside(p):
    z0, z1 = HEAD_ROWS[-1][0], HEAD_ROWS[0][0]
    if not (z0 < p.z < z1):
        return False
    hw = _cubic(HEAD_ROWS, p.z, 1)
    f, b = _cubic(HEAD_ROWS, p.z, 2), _cubic(HEAD_ROWS, p.z, 3)
    e = _cubic(HEAD_ROWS, p.z, 4)
    if hw <= 1e-5 or b - f <= 1e-5:
        return False
    ym, hd = (f + b) / 2, (b - f) / 2
    t = max(0.0, min(1.0, (p.y - f) / (b - f)))
    hw *= 1.0 + _cubic(HEAD_JAW, p.z, 1) * t * t * (3 - 2 * t)
    ey = e + 0.6 if p.y < ym else e - 0.3       # flatter face, rounder back of the skull
    return (abs(p.x) / hw) ** e + (abs(p.y - ym) / hd) ** ey <= 1.0


def head_surface(d):
    """Where the ray from HEAD_CENTRE along d leaves the head."""
    lo, step = 0.0, 0.004
    hi = step
    while head_inside(HEAD_CENTRE + d * hi):
        lo, hi = hi, hi + step
    for _ in range(30):
        mid = (lo + hi) / 2
        if head_inside(HEAD_CENTRE + d * mid):
            lo = mid
        else:
            hi = mid
    return HEAD_CENTRE + d * lo


def head_field(p):
    """Signed-ish field of the head profile: < 0 inside, 0 on the surface (smooth in x, y and z)."""
    z0, z1 = HEAD_ROWS[-1][0], HEAD_ROWS[0][0]
    z = max(z0 + 1e-5, min(z1 - 1e-5, p.z))
    hw = max(1e-4, _cubic(HEAD_ROWS, z, 1))
    f, b = _cubic(HEAD_ROWS, z, 2), _cubic(HEAD_ROWS, z, 3)
    e = _cubic(HEAD_ROWS, z, 4)
    ym, hd = (f + b) / 2, max(1e-4, (b - f) / 2)
    t = max(0.0, min(1.0, (p.y - f) / max(1e-4, b - f)))
    hw *= 1.0 + _cubic(HEAD_JAW, z, 1) * t * t * (3 - 2 * t)
    ey = e + 0.6 if p.y < ym else e - 0.3
    return ((abs(p.x) / hw) ** e + (abs(p.y - ym) / hd) ** ey) ** (1.0 / min(e, ey)) - 1.0


def head_normals(ob):
    """Give the head the normals of its smooth profile surface (finite-difference gradient of
    head_field) instead of averaged face normals: the cel bands then follow the form, not the grid.
    The topmost and bottommost vertices, where the field degenerates, keep their face normals."""
    me = ob.data
    gi = ob.vertex_groups["Head"].index
    head = {v.index for v in me.vertices if any(g.group == gi for g in v.groups)}
    normals = [Vector(cn.vector) for cn in me.corner_normals]
    z0, z1 = HEAD_ROWS[-1][0], HEAD_ROWS[0][0]
    h = 2e-4
    cache = {}
    for li, loop in enumerate(me.loops):
        vi = loop.vertex_index
        if vi not in head:
            continue
        if vi not in cache:
            p = me.vertices[vi].co.copy()
            if p.z > z1 - 0.004 or p.z < z0 + 0.003:
                cache[vi] = None
            else:
                g = Vector([(head_field(p + d * h) - head_field(p - d * h)) for d in (X, Y, Z)])
                cache[vi] = g.normalized() if g.length > 0 else None
        if cache[vi] is not None:
            normals[li] = cache[vi]
    me.normals_split_custom_set(normals)


def build_head():
    """A soft rounded block: a cube-sphere grid (quads only, no poles) pushed out to the profile
    surface. Rays are spread by the head's own proportions so the quads stay even."""
    sh = Shell("Head", "Head")
    bm = sh.bm
    n = HEAD_GRID[LOD]
    bmesh.ops.create_cube(bm, size=2.0)
    bmesh.ops.subdivide_edges(bm, edges=bm.edges[:], cuts=n - 1, use_grid_fill=True)
    ext = Vector((0.186, 0.156, 0.193))
    for v in bm.verts:
        c = v.co
        q = Vector([math.tan(math.pi / 4 * max(-1.0, min(1.0, c[i]))) * ext[i] for i in range(3)])
        v.co = head_surface(q.normalized())
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return sh


def build_neck():
    """A short, round neck about 0.29 head widths across. Its top is hidden in the head's underside,
    its foot spreads into the trapezius rings of the torso."""
    sh = Shell("Neck", "Head")
    rows = [
        (1.080, 0.066, -0.052, 0.072, 0.040),
        (1.110, 0.058, -0.045, 0.064, 0.040),
        (1.140, 0.054, -0.041, 0.060, 0.040),
        (1.245, 0.051, -0.039, 0.057, 0.038),
    ]
    rows = keep(rows, lod2=[0, 3])
    loft(sh, vertical_stations(rows, seg=sg(3)) if LOD < 2 else
         [station((0, 0, z), X, Y, box_pts(hw, f, b)) for z, hw, f, b, _r in rows])
    return sh


def build_torso():
    """Rib cage: a clear chest (deepest at 0.99 m, with a plane break under it at 0.95 m), a narrow
    waist, and a shoulder line that rises to the trapezius and turns over into the neck. A centre
    line runs down the chest and the spine."""
    sh = Shell("Torso", "Torso")
    rows = [
        (0.800, 0.080, -0.082, 0.054, 0.022, 0.000, 0.000),   # hidden in the pelvis
        (0.850, 0.088, -0.089, 0.061, 0.024, 0.000, 0.000),
        (0.885, 0.092, -0.095, 0.065, 0.025, 0.002, 0.002),
        (0.918, 0.099, -0.104, 0.069, 0.026, 0.003, 0.003),
        (0.952, 0.108, -0.112, 0.077, 0.028, 0.005, 0.003),   # under-chest plane break
        (0.990, 0.118, -0.116, 0.091, 0.030, 0.006, 0.004),
        (1.025, 0.125, -0.109, 0.098, 0.031, 0.005, 0.004),
        (1.055, 0.128, -0.090, 0.096, 0.031, 0.004, 0.004),
        (1.080, 0.126, -0.066, 0.089, 0.030, 0.002, 0.003),   # top of the shoulder
        (1.096, 0.104, -0.054, 0.080, 0.028, 0.001, 0.002),
        (1.109, 0.078, -0.048, 0.071, 0.025, 0.000, 0.000),   # trapezius
        (1.126, 0.060, -0.043, 0.063, 0.021, 0.000, 0.000),   # meets the neck
    ]
    sharp = (4,)
    if LOD == 1:
        rows, sharp = [rows[i] for i in (0, 1, 2, 4, 5, 6, 7, 8, 9, 10)], (3,)
    if LOD == 2:
        rows, sharp = [rows[i] for i in (0, 4, 7, 10)], (1,)
    loft(sh, vertical_stations(rows, seg=sg(2), ridge=LOD < 2), sharp=sharp,
         sharp_cols=(6, 13) if LOD == 0 else ())
    return sh


def build_pelvis():
    """Pelvis block: belt line at 0.835 m, the hip line at 0.79 m, then the concept's brief: the
    sides run in a straight V down to the crotch, so in front and behind the thighs come out from
    under two diagonal creases. The hip joints are balls (the thigh domes) inside it."""
    sh = Shell("Pelvis", "Torso")
    rows = [
        (0.655, 0.022, -0.030, 0.034, 0.010, 0.000, 0.000),
        (0.668, 0.034, -0.052, 0.060, 0.014, 0.000, 0.000),
        (0.685, 0.046, -0.069, 0.078, 0.018, 0.001, 0.001),
        (0.705, 0.059, -0.082, 0.090, 0.020, 0.002, 0.002),
        (0.730, 0.074, -0.090, 0.097, 0.022, 0.003, 0.003),   # seat
        (0.760, 0.090, -0.094, 0.095, 0.024, 0.003, 0.003),
        (0.790, 0.102, -0.096, 0.085, 0.026, 0.003, 0.002),   # hip line
        (0.812, 0.101, -0.097, 0.074, 0.025, 0.002, 0.002),
        (0.835, 0.092, -0.095, 0.066, 0.024, 0.002, 0.001),   # belt line
        (0.880, 0.087, -0.090, 0.060, 0.022, 0.000, 0.000),
    ]
    sharp = (8,)
    if LOD == 1:
        rows, sharp = [rows[i] for i in (0, 1, 3, 4, 5, 6, 8, 9)], (6,)
    if LOD == 2:
        rows, sharp = [rows[i] for i in (0, 3, 6, 8)], ()
    loft(sh, vertical_stations(rows, seg=sg(2), ridge=LOD < 2), sharp=sharp,
         sharp_cols=(6, 13) if LOD == 0 else ())
    return sh


def build_upper_arm():
    """Shoulder dome (centred on the shoulder pivot: the deltoid cap) tapering down to a half
    cylinder round the elbow hinge."""
    sh = Shell("UpperArm.L", "Arms", groups=("Arm.L",))
    ax = lateral(UPPER_DIR)
    L = (ELBOW - SHOULDER).length
    rows = dome_rows(0.048, 0.050, 0.017, (78, 58, 34))
    rows += [(0.0, 0.048, 0.050, 0.017), (0.93 * L, 0.039, 0.041, 0.013)]
    rows += [(L + s, hw, hd, r) for s, hw, hd, r in hinge_rows(0.039, 0.041, 0.013, (0, 40, 68))]
    rows = keep(rows, lod2=[1, 5, 7])
    loft(sh, axis_stations(SHOULDER, UPPER_DIR, ax, rows, sg(2)))
    return sh


def build_lower_arm():
    """Forearm block: a rim just outside the upper arm's elbow cylinder, the widest section just
    below the elbow, then a long taper that flattens toward the wrist and turns with the hand."""
    sh = Shell("LowerArm.L", "Arms", groups=("Arm.L",))
    ax = lateral(LOWER_DIR)
    pts = [ELBOW - LOWER_DIR * 0.010, ELBOW, WRIST, WRIST + LOWER_DIR * 0.026]
    specs = [
        (0, 0.0, 0.044, 0.045, 0.012),     # bevelled rim
        (0, 0.75, 0.050, 0.050, 0.015),    # forearm step (the concept's elbow line)
        (1, 0.12, 0.053, 0.051, 0.016),
        (1, 0.40, 0.051, 0.048, 0.016),
        (1, 0.78, 0.045, 0.041, 0.014, 0.0, 0.0, 10),
        (2, 0.0, 0.039, 0.035, 0.013, 0.0, 0.0, 22),    # wrist loop, turning toward the hand
        (2, 1.0, 0.036, 0.032, 0.012, 0.0, 0.0, 30),
    ]
    specs = keep(specs, lod2=[0, 1, 5])
    loft(sh, path_stations(pts, specs, ax, seg=sg(2)), sharp=(1,) if LOD < 2 else ())
    return sh


def build_palm():
    """A boxy palm, thicker at the knuckles, with the concept's line down the back of the hand."""
    sh = Shell("Palm.L", "Hands", groups=("Hand.L",))
    a, b, c = hand_frame()
    # s along a, half breadth (b), half thickness (c), back ridge.
    rows = [(0.004, 0.034, 0.020, 0.000), (0.019, 0.042, 0.025, 0.001), (0.039, 0.050, 0.029, 0.003),
            (0.071, 0.056, 0.030, 0.003), (0.097, 0.057, 0.028, 0.002), (0.109, 0.054, 0.023, 0.000)]
    rows = keep(rows, lod2=[0, 3, 5])
    sts = []
    for s, hb, hc, rg in rows:
        o = H(s, 0.0, PALM_N0)
        pts = [(pu * HAND_SCALE, pv * HAND_SCALE)
               for pu, pv in (rrect(hb, -hc, hc, 0.008, sg(2), (rg, 0.0)) if LOD < 2 else box_pts(hb, -hc, hc))]
        sts.append(station(o, b, c, pts))
    nring = len(sts[0]["pts"])
    loft(sh, sts, sharp=(2,), sharp_cols=(nring - 1,) if LOD < 2 else ())
    return sh


def finger_rows(L, hw, hd):
    """Rings along a finger: base inside the palm, a joint ring at each knuckle with a ring 3 mm to
    each side (the rig blends only the joint ring: a short, blocky bend), and a bevelled tip."""
    L1, L2, L3 = L
    j1, j2 = L1, L1 + L2
    t = L1 + L2 + L3
    k = 0.003
    rows = [(-0.012, hw * 0.96, hd * 0.95), (0.0, hw, hd),
            (j1 - k, hw, hd * 0.98), (j1, hw * 0.99, hd * 0.97), (j1 + k, hw * 0.98, hd * 0.96),
            (j2 - k, hw * 0.97, hd * 0.93), (j2, hw * 0.96, hd * 0.92), (j2 + k, hw * 0.95, hd * 0.91),
            (t, hw * 0.80, hd * 0.74)]
    return keep(rows, lod1=[0, 3, 6, 8], lod2=[0, 3, 6, 8])


def build_finger(name):
    sh = Shell(name + ".L", "Hands", groups=("Hand.L",))
    a, b, c = hand_frame()
    w, ls, ws = FINGERS[name]
    o = H(KNUCKLE_S, w, FINGER_N)
    L = [x * ls * HAND_SCALE for x in FINGER_L]
    rows = finger_rows(L, FINGER_HW * ws * HAND_SCALE, FINGER_HD * HAND_SCALE)
    sts = [station(o + a * s, b, c, rrect(hw, -hd, hd, 0.0042 * HAND_SCALE, 1)) for s, hw, hd in rows]
    loft(sh, sts)
    return sh


def build_mitten():
    """LOD2: the four fingers as one block (weighted to the middle finger)."""
    sh = Shell("Middle.L", "Hands", groups=("Hand.L",))
    a, b, c = hand_frame()
    o = H(KNUCKLE_S, 0.0, FINGER_N)
    L = [x * 0.95 * HAND_SCALE for x in FINGER_L]
    rows = finger_rows(L, 0.058 * HAND_SCALE, FINGER_HD * HAND_SCALE)
    sts = [station(o + a * s, b, c, box_pts(hw, -hd, hd)) for s, hw, hd in rows]
    loft(sh, sts)
    return sh


def build_thumb():
    sh = Shell("Thumb.L", "Hands", groups=("Hand.L",))
    a, b, c = hand_frame()
    q = thumb_points()
    S = HAND_SCALE
    specs = [
        (0, 0.0, 0.0165 * S, 0.0175 * S, 0.0045 * S),
        (1, 0.0, 0.0185 * S, 0.0195 * S, 0.0055 * S),    # CMC joint ring
        (1, 0.92, 0.0178 * S, 0.0188 * S, 0.0052 * S),
        (2, 0.0, 0.0175 * S, 0.0185 * S, 0.0052 * S),    # MCP joint ring
        (2, 0.09, 0.0172 * S, 0.0182 * S, 0.0050 * S),
        (2, 0.90, 0.0166 * S, 0.0174 * S, 0.0048 * S),
        (3, 0.0, 0.0162 * S, 0.0170 * S, 0.0048 * S),    # IP joint ring
        (3, 0.10, 0.0160 * S, 0.0166 * S, 0.0046 * S),
        (3, 0.80, 0.0150 * S, 0.0152 * S, 0.0044 * S),
        (3, 1.0, 0.0105 * S, 0.0105 * S, 0.0030 * S),
    ]
    specs = keep(specs, lod1=[0, 1, 3, 6, 9], lod2=[1, 3, 9])
    sts = path_stations(q, specs, c, seg=1)
    if LOD == 2:
        for st, sp in zip(sts, specs):
            st["pts"] = box_pts(sp[2], -sp[3], sp[3])
    loft(sh, sts)
    return sh


def build_thigh():
    """Thigh: a ball-joint dome centred on the hip pivot (it turns inside itself and carries the
    hip's outer line), full at the top, a steady taper to the knee, and a half cylinder round the
    knee hinge."""
    sh = Shell("Thigh.L", "Legs", groups=("Leg.L",))
    ax = lateral(THIGH_DIR)
    L = (KNEE - HIP).length
    rows = [(-0.058 * math.sin(math.radians(a)), max(0.006, 0.070 * math.cos(math.radians(a))),
             max(0.006, 0.082 * math.cos(math.radians(a))), 0.020 * max(0.35, math.cos(math.radians(a))))
            for a in (80, 60, 35)]
    rows += [(0.0, 0.070, 0.082, 0.020), (0.15 * L, 0.073, 0.083, 0.021),
             (0.58 * L, 0.061, 0.067, 0.018), (0.93 * L, 0.051, 0.058, 0.017)]
    rows += [(L + s, hw, hd, r) for s, hw, hd, r in hinge_rows(0.051, 0.057, 0.017, (0, 40, 68))]
    rows = keep(rows, lod1=[0, 2, 3, 4, 5, 6, 7, 9], lod2=[0, 3, 9])
    loft(sh, axis_stations(HIP, THIGH_DIR, ax, rows, sg(2)))
    return sh


def build_shin():
    """Lower leg: a rim just outside the thigh's knee cylinder (the knee line), then a prism that
    widens toward the boot and fills out a little at the back, as on the sheet."""
    sh = Shell("Shin.L", "Legs", groups=("Leg.L",))
    ax = lateral(SHIN_DIR)
    pts = [KNEE - SHIN_DIR * 0.010, KNEE, ANKLE]
    specs = [
        (0, 0.0, 0.052, 0.059, 0.015),
        (0, 0.85, 0.054, 0.061, 0.016),                # the knee line, flush with the thigh's end
        (1, 0.10, 0.055, 0.062, 0.017, 0.0, -0.002),
        (1, 0.45, 0.061, 0.070, 0.019, 0.0, -0.007),   # dv < 0: toward the back
        (1, 0.74, 0.067, 0.075, 0.020, 0.0, -0.007),
        (1, 1.0, 0.070, 0.077, 0.020, 0.0, -0.005),
    ]
    specs = keep(specs, lod2=[0, 1, 5])
    loft(sh, path_stations(pts, specs, ax, seg=sg(2)), sharp=(1,) if LOD < 2 else ())
    return sh


BOOT_W = 1.22  # width scale of the sole, pads and toe box (v2: 1.34, too heavy next to the new legs)


def boot_footprint():
    """Sole outline in boot space (lx outward, ly forward), counter-clockwise from the heel."""
    pts = [(-0.044, -0.106), (0.048, -0.106), (0.070, -0.090), (0.082, -0.009), (0.100, 0.097),
           (0.092, 0.170), (0.066, 0.196), (-0.056, 0.196), (-0.080, 0.170), (-0.088, 0.097),
           (-0.072, -0.009), (-0.066, -0.090)]
    return [(x * BOOT_W, y) for x, y in pts]


# Tread pads under the sole: heel and forefoot, with the arch band between them.
HEEL_PAD = [(-0.036, -0.095), (0.040, -0.095), (0.058, -0.080), (0.066, -0.024), (-0.060, -0.024),
            (-0.058, -0.080)]
FORE_PAD = [(-0.066, 0.032), (0.076, 0.032), (0.088, 0.097), (0.079, 0.156), (0.053, 0.179),
            (-0.043, 0.179), (-0.067, 0.156), (-0.076, 0.097)]


def build_boot():
    """Boot: tread pads, a sole slab with chamfered edges, and a wedge upper. The cuff is a level
    ring just outside the shin (the concept's ankle seam); the instep runs down at about 35 degrees
    to a low, bevelled toe box."""
    sh = Shell("Boot.L", "Feet")
    cx, cy = BOOT_CX, BOOT_CY

    def w(lx, ly, z):
        return Vector((cx + lx, cy - ly, z))

    if LOD == 0:
        for pad in ([(x * BOOT_W, y) for x, y in HEEL_PAD], [(x * BOOT_W, y) for x, y in FORE_PAD]):
            rows = [(0.000, -0.003), (0.003, 0.0), (0.010, 0.0)]
            loft(sh, [station((cx, cy, z), X, FRONT, offset_poly(pad, d)) for z, d in rows])
    fp = boot_footprint()
    if LOD == 2:
        fp = fp[::2]
    rows = [(0.005, -0.005), (0.012, 0.0), (0.044, 0.0), (0.053, -0.007)]
    if LOD == 2:
        rows = [(0.000, -0.004), (0.048, 0.0)]
    # The end rings are sharp: otherwise the big flat caps pull the chamfer normals toward
    # straight down and the sole edge renders as a dark band in the engine.
    loft(sh, [station((cx, cy, z), X, FRONT, offset_poly(fp, d)) for z, d in rows], sharp=(0, len(rows) - 1))

    up = [w(lx, ly, 0.046) for lx, ly in fp]
    # The cuff follows the leg, which leans in: its centre is about 2.4 cm inside cx and 0.8 cm
    # ahead of the ankle. It is 5 mm outside the shin all round.
    lc, lf = -0.024, 0.008
    k = BOOT_W
    up += [w(-0.048 * k, -0.097, 0.112), w(0.050 * k, -0.097, 0.112),
           w(lc - 0.068, lf - 0.084, 0.180), w(lc + 0.068, lf - 0.084, 0.180),
           w(lc - 0.075, lf, 0.184), w(lc + 0.075, lf, 0.184),
           w(lc - 0.073, lf + 0.074, 0.176), w(lc + 0.073, lf + 0.074, 0.176),
           w(-0.066 * k, 0.108, 0.148), w(0.076 * k, 0.108, 0.148),
           w(-0.068 * k, 0.152, 0.110), w(0.080 * k, 0.152, 0.110),
           w(-0.062 * k, 0.190, 0.076), w(0.072 * k, 0.190, 0.076)]
    up_shell = Shell("tmp", "Feet")
    hull_shell(up_shell, up, bevel=0.007, segments=(1 if LOD == 0 else 0))
    # A loop across the ball of the foot for the toes bone.
    bm = up_shell.bm
    bmesh.ops.bisect_plane(bm, geom=bm.verts[:] + bm.edges[:] + bm.faces[:],
                           plane_co=w(0, BALL_LY, 0), plane_no=FRONT)
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
        m = bpy.data.materials.get("M_" + s) or bpy.data.materials.new("M_" + s)
        bsdf = next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        bsdf.inputs["Base Color"].default_value = GREY
        bsdf.inputs["Roughness"].default_value = 0.65
        m.diffuse_color = GREY
        mats[s] = m
    return mats


def build_parts():
    shells = [build_head(), build_neck(), build_torso(), build_pelvis()]
    paired = [build_upper_arm(), build_lower_arm(), build_palm(), build_thumb(),
              build_thigh(), build_shin(), build_boot()]
    if LOD < 2:
        paired += [build_finger(f) for f in FINGERS]
    else:
        paired.append(build_mitten())
    return shells, paired


def build(glb=None, blend=None, lod=0, head_seg=None, reset=True, name="DefaultAvatar"):
    """Build one LOD as a joined object. Each shell survives as a vertex group of its own name
    (plus Arm/Hand/Leg groups per side for close-ups)."""
    global LOD, HEAD_SEG
    LOD = lod
    if head_seg is not None:
        HEAD_SEG = head_seg
    if reset:
        bpy.ops.wm.read_factory_settings(use_empty=True)
    shells, paired = build_parts()
    objs = [(shell_to_object(s), s) for s in shells]
    for s in paired:
        objs.append((shell_to_object(s, name=s.name), s))
        objs.append((shell_to_object(s, mirror=True, name=s.name[:-2] + ".R"), s))

    mats = make_materials()
    for ob, s in objs:
        for slot in SLOTS:
            ob.data.materials.append(mats[slot])
        idx = SLOTS.index(s.slot)
        for p in ob.data.polygons:
            p.material_index = idx
        side = ob.name[-2:] if ob.name[-2:] in (".L", ".R") else ""
        for g in (ob.name,) + tuple(gr[:-2] + side for gr in s.groups):
            vg = ob.vertex_groups.new(name=g)
            vg.add(list(range(len(ob.data.vertices))), 1.0, "REPLACE")
    for s in shells + paired:
        s.bm.free()

    # Join into one mesh; the part names live on as vertex groups.
    bpy.ops.object.select_all(action="DESELECT")
    for ob, _ in objs:
        ob.select_set(True)
    root = objs[0][0]
    bpy.context.view_layer.objects.active = root
    bpy.ops.object.join()
    root.name = name
    root.data.name = name

    # Face-area weighted normals: big planes flat, bevel strips soft, marked rings crisp.
    mod = root.modifiers.new("WeightedNormal", "WEIGHTED_NORMAL")
    mod.mode = "FACE_AREA"
    mod.weight = 80
    mod.keep_sharp = True
    bpy.ops.object.modifier_apply(modifier=mod.name)
    head_normals(root)

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
    print(f"[avatar] lod={lod} triangles={tris} vertices={len(root.data.vertices)}")
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
    ap.add_argument("--glb", default=os.path.join(here, "out", "default_avatar_mesh.glb"))
    ap.add_argument("--blend", default=None)
    ap.add_argument("--no-glb", action="store_true")
    ap.add_argument("--lod", type=int, default=0)
    ap.add_argument("--head-seg", type=int, default=None)
    a = ap.parse_args(argv)
    build(None if a.no_glb else a.glb, a.blend, a.lod, a.head_seg)


if __name__ == "__main__":
    main()
