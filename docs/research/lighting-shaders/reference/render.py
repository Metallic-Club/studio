"""Offline reference renders for issue #37. Writes small PNGs to docs/agents/37-lighting/.

    python docs/research/lighting-shaders/reference/render.py            # everything (about 2-3 min)
    python docs/research/lighting-shaders/reference/render.py sky        # one group: sky | curves | presets | table

What it models (and the engine plan follows the same maths):
- Sky: Hillaire 2020 LUTs (atmosphere.py), sun and moon as two lights, sun/moon disk, stars.
- Scene: axis-aligned Parts on a ground plane, sun with cone-sampled soft shadows (stand-in for PCSS),
  sky irradiance integrated from the sky-view LUT (stand-in for SH ambient) times a ray-traced sky
  visibility (stand-in for voxel sky visibility + GTAO), point lights with the windowed falloff, Neon.
- World fog: analytic exponential height fog coloured from the sky-view LUT (height_fog.wgsl).
- Post: auto exposure (log-average, centre weighted, percentile band, night bias), bloom (lerp 0.04),
  grading, tonemapper from the preset, vignette, sRGB.
It has no GI bounce and no textures, so interiors look darker and flatter than the engine will.
"""
from __future__ import annotations

import math
import os
import sys
import time

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(__file__))
import atmosphere as A  # noqa: E402
import presets as P  # noqa: E402
import tonemap as TM  # noqa: E402

OUT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", "..", "..", "agents", "37-lighting"))
SUN_TOA_LUX = 120000.0  # sun illuminance at the top of the atmosphere for Brightness 1
MOON_LUX = 0.25  # full moon illuminance for MoonBrightness 1
STUD_M = 0.28  # metres per stud
LUM = np.array([0.2126, 0.7152, 0.0722])
rng = np.random.default_rng(37)


# ---------------------------------------------------------------------------------------------
# Sun and moon from ClockTime and GeographicLatitude (equinox: sunrise 6:00, sunset 18:00).

def celestial_dirs(clock: float, lat_deg: float, declination_deg: float = 0.0):
    """World directions (x east, y up, z north) toward the sun and the moon (moon opposite the sun)."""
    phi = math.radians(lat_deg)
    dec = math.radians(declination_deg)
    h = math.radians(15.0 * (clock - 12.0))
    east = -math.cos(dec) * math.sin(h)
    north = math.cos(phi) * math.sin(dec) - math.sin(phi) * math.cos(dec) * math.cos(h)
    up = math.sin(phi) * math.sin(dec) + math.cos(phi) * math.cos(dec) * math.cos(h)
    sun = np.array([east, up, north])
    sun /= np.linalg.norm(sun)
    return sun, -sun


def elevation_deg(d):
    return math.degrees(math.asin(max(-1.0, min(1.0, d[1]))))


# ---------------------------------------------------------------------------------------------
# Atmosphere from creator properties.

def srgb01(c):
    return np.array(c, dtype=np.float64) / 255.0


def lin(c):
    return TM.srgb_decode(srgb01(c))


def atmosphere_for(p) -> A.Atmosphere:
    atm = A.Atmosphere()
    mie_scale = 1.0 + 0.8 * p["Haze"]
    tint = lin(p["Color"]) / lin(P.DEFAULTS["Color"])
    atm.mie_scattering = atm.mie_scattering * mie_scale * np.clip(tint, 0.0, 1.1)
    atm.mie_extinction = np.maximum(atm.mie_extinction * mie_scale, atm.mie_scattering)
    atm.mie_g = min(0.76 + 0.019 * p["Glare"], 0.95)
    atm.rayleigh_scattering = atm.rayleigh_scattering * (lin(p["AirColor"]) / lin(P.DEFAULTS["AirColor"]))
    return atm


_cache = {}


def luts_for(p):
    key = (p["Haze"], p["Glare"], tuple(p["Color"]), tuple(p["AirColor"]))
    if key not in _cache:
        atm = atmosphere_for(p)
        lut_t = A.build_transmittance(atm)
        lut_ms = A.build_multiscatter(atm, lut_t)
        _cache[key] = (atm, lut_t, lut_ms)
    return _cache[key]


def to_local(d):
    """World (x east, y up, z north) -> atmosphere frame (+z up)."""
    return np.stack([d[..., 0], d[..., 2], d[..., 1]], -1)


class Sky:
    """Sky-view LUTs for the sun and the moon, scaled to relative illuminance (sun TOA at Brightness 1 = 1)."""

    def __init__(self, p, sun_elevation=None):
        self.p = p
        self.atm, self.lut_t, self.lut_ms = luts_for(p)
        self.sun, self.moon = celestial_dirs(p["ClockTime"], p["GeographicLatitude"])
        if sun_elevation is not None:  # west-facing sun at an exact elevation (for tables and sky strips)
            e = math.radians(sun_elevation)
            self.sun = np.array([-math.cos(e), math.sin(e), 0.0])
            self.moon = -self.sun
        self.r = self.atm.bottom_radius + 0.0002
        self.e_sun = 1.0  # sky uses the physical sun; Brightness only scales direct sun light (see doc)
        self.e_moon = MOON_LUX / SUN_TOA_LUX * p["MoonBrightness"]
        self.sv_sun = self._skyview(self.sun)
        self.sv_moon = self._skyview(self.moon)
        self.cover = 0.0
        self.l_overcast = 0.0
        if p["CloudCover"] > 0:
            # Overcast sky: CIE overcast distribution L = Lz (1 + 2 sin(elev)) / 3 holding 35% of the clear-day
            # horizontal illuminance (sun + sky). E_h = Lz * 7 pi / 9.
            e_clear = horizontal_illuminance(self, clear=True)
            self.cover = p["CloudCover"]
            self.l_overcast = 0.35 * e_clear * 9.0 / (7.0 * math.pi)

    def _skyview(self, d):
        e = elevation_deg(d)
        lut, _ = A.build_skyview(self.atm, self.lut_t, self.lut_ms, e)
        return lut

    def radiance(self, dirs):
        """Sky radiance for world directions [...,3] (no sun disk)."""
        out = np.zeros(dirs.shape)
        for lut, d, e in ((self.sv_sun, self.sun, self.e_sun), (self.sv_moon, self.moon, self.e_moon)):
            ld = to_local(dirs)
            # The LUT frame has the light at azimuth 0; rotate directions so azimuth is relative to it.
            sd = to_local(d)
            out += e * A.sample_skyview(self.atm, lut, self.r, ld, sd)
        if self.cover > 0:
            ov = self.l_overcast * ((1.0 + 2.0 * np.clip(dirs[..., 1], 0, 1)) / 3.0)[..., None]
            k = self.cover ** 0.7
            out = out * (1 - k) + ov * k
        return out

    def light_at_ground(self, d, e):
        t = A.sun_transmittance(self.atm, self.lut_t, elevation_deg(d))
        return e * t

    def sun_light(self, clear=False):
        cover = 0.0 if clear else self.cover
        return self.light_at_ground(self.sun, self.e_sun) * self.p["Brightness"] * (1 - cover) ** 1.5

    def moon_light(self, clear=False):
        cover = 0.0 if clear else self.cover
        return self.light_at_ground(self.moon, self.e_moon) * self.p["Brightness"] * (1 - cover) ** 1.5


def fib_sphere(n):
    i = np.arange(n) + 0.5
    phi = np.arccos(1 - 2 * i / n)
    th = math.pi * (1 + 5 ** 0.5) * i
    return np.stack([np.cos(th) * np.sin(phi), np.cos(phi), np.sin(th) * np.sin(phi)], -1)


def irradiance_table(sky: Sky, normals, ground_albedo=0.2):
    """Irradiance from sky (upper) and from a ground of given albedo (lower) for each normal."""
    dirs = fib_sphere(4096)
    up = dirs[:, 1] > 0
    L = np.zeros(dirs.shape)
    L[up] = sky.radiance(dirs[up])
    # ground radiance: lit by sun, moon and sky on a horizontal plane
    e_h = horizontal_illuminance(sky)
    L[~up] = ground_albedo * e_h / math.pi * (1.0 + srgb01(sky.p["ColorShift_Bottom"]))
    dw = 4 * math.pi / len(dirs)
    out = []
    for n in normals:
        c = np.clip(dirs @ n, 0, None)
        out.append((L * c[:, None]).sum(0) * dw)
    return np.array(out)


def horizontal_illuminance(sky: Sky, include_sky=True, clear=False):
    e = sky.sun_light(clear) * max(sky.sun[1], 0) + sky.moon_light(clear) * max(sky.moon[1], 0)
    if include_sky:
        e = e + sky_horizontal(sky)
    return e


def sky_horizontal(sky: Sky):
    dirs = fib_sphere(4096)
    dirs = dirs[dirs[:, 1] > 0]
    return (sky.radiance(dirs) * dirs[:, 1:2]).sum(0) * (2 * math.pi / len(dirs))


# ---------------------------------------------------------------------------------------------
# Scene and ray casting.

Box = tuple  # (min[3], max[3], color sRGB, roughness, neon)


def scene_outdoor():
    b = []

    def add(c, s, col, rough=0.55, neon=0.0):
        c = np.array(c, float)
        s = np.array(s, float) / 2
        b.append((c - s, c + s, col, rough, neon))

    add((0, 7, 22), (44, 14, 2), (163, 162, 165))  # wall
    add((-9, 3, 4), (6, 6, 6), (196, 40, 28))  # red cube
    add((1, 5, 10), (4, 10, 4), (13, 105, 172))  # blue tower
    add((10, 1.5, 2), (8, 3, 8), (245, 205, 48))  # yellow slab
    add((-2, 0.5, -6), (5, 1, 5), (242, 243, 243))  # white step
    add((16, 10, 14), (6, 20, 6), (91, 93, 105))  # dark tower
    add((-16, 4, 12), (2, 8, 2), (0, 255, 255), 0.3, 1.0)  # cyan neon pillar
    return b


def scene_interior():
    b = []

    def add(c, s, col, rough=0.5, neon=0.0):
        c = np.array(c, float)
        s = np.array(s, float) / 2
        b.append((c - s, c + s, col, rough, neon))

    add((0, 16, 10), (44, 1, 60), (60, 62, 70))  # ceiling
    add((-21, 8, 10), (1, 16, 60), (120, 124, 135))  # left wall
    add((21, 8, 10), (1, 16, 60), (120, 124, 135))  # right wall
    add((0, 8, 38), (44, 16, 1), (90, 94, 104))  # back wall
    add((0, 8, -20), (44, 16, 1), (90, 94, 104))  # wall behind camera (with no window)
    add((0, 15.2, 10), (2, 0.4, 40), (190, 220, 255), 0.3, 1.0)  # ceiling light strip
    add((-20.3, 3, 14), (0.4, 0.6, 30), (120, 255, 230), 0.3, 1.0)  # floor strip left
    add((20.3, 3, 14), (0.4, 0.6, 30), (255, 140, 90), 0.3, 1.0)  # floor strip right
    add((0, 2, 18), (10, 4, 6), (200, 202, 210), 0.25)  # console
    add((-8, 1.5, 8), (3, 3, 3), (196, 40, 28))
    add((8, 3, 26), (4, 6, 4), (13, 105, 172))
    return b


def trace(o, d, boxes, t_max=1e5, ground=True):
    n = d.shape[0]
    t_best = np.full(n, t_max)
    idx = np.full(n, -1)
    nrm = np.zeros((n, 3))
    with np.errstate(divide="ignore", invalid="ignore"):
        inv = 1.0 / d
        for i, (bmin, bmax, *_rest) in enumerate(boxes):
            t0 = (bmin - o) * inv
            t1 = (bmax - o) * inv
            tn = np.minimum(t0, t1)
            tf = np.maximum(t0, t1)
            t_near = tn.max(1)
            t_far = tf.min(1)
            hit = (t_far >= np.maximum(t_near, 0)) & (t_near > 1e-4) & (t_near < t_best)
            if hit.any():
                t_best = np.where(hit, t_near, t_best)
                idx = np.where(hit, i, idx)
                axis = tn.argmax(1)
                nn = np.zeros((n, 3))
                nn[np.arange(n), axis] = -np.sign(d[np.arange(n), axis])
                nrm = np.where(hit[:, None], nn, nrm)
        if ground:
            tg = np.where(d[:, 1] < 0, -o[:, 1] / d[:, 1], np.inf)
            hit = (tg > 1e-4) & (tg < t_best)
            t_best = np.where(hit, tg, t_best)
            idx = np.where(hit, -2, idx)
            nrm = np.where(hit[:, None], np.array([0.0, 1.0, 0.0]), nrm)
    return t_best, idx, nrm


def occluded(o, d, boxes, t_max=1e5):
    t, idx, _ = trace(o, d, boxes, t_max=t_max, ground=False)
    return (idx >= 0) & (t < t_max)


def cone_dirs(center, half_angle, k):
    """k directions inside a cone (for soft shadows)."""
    c = center / np.linalg.norm(center)
    a = np.array([0, 1, 0]) if abs(c[1]) < 0.9 else np.array([1, 0, 0])
    t = np.cross(a, c)
    t /= np.linalg.norm(t)
    b = np.cross(c, t)
    out = []
    for i in range(k):
        r = math.sqrt((i + 0.5) / k) * math.tan(half_angle)
        th = i * 2.399963
        v = c + t * r * math.cos(th) + b * r * math.sin(th)
        out.append(v / np.linalg.norm(v))
    return out


def basis(n):
    a = np.where(np.abs(n[:, 1:2]) < 0.9, np.array([0, 1.0, 0]), np.array([1.0, 0, 0]))
    t = np.cross(a, n)
    t /= np.linalg.norm(t, axis=1, keepdims=True)
    return t, np.cross(n, t)


# ---------------------------------------------------------------------------------------------
# Height fog (same maths as height_fog.wgsl).

def fog_sigma0(p):
    return 0.012 * p["Density"] ** 2


def height_fog(p, o, d, dist):
    """Returns (transmittance [N,3], opacity weight [N,1]) along camera rays."""
    s0 = fog_sigma0(p)
    f = p["HeightFalloff"]
    absorb = 1.0 + 3.0 * srgb01(p["Absorption"])
    base = s0 * math.exp(-f * (o[0, 1] - p["FogHeight"])) if f > 0 else s0
    if f > 0:
        k = f * d[:, 1] * dist
        with np.errstate(over="ignore", invalid="ignore"):
            integ = np.where(np.abs(k) > 1e-4, (1.0 - np.exp(-k)) / (f * d[:, 1]), dist)
    else:
        integ = dist
    tau = base * integ[:, None] * absorb
    T = np.exp(-tau)
    max_op = 1.0 - 0.75 * p["Offset"]
    T = 1.0 - (1.0 - T) * max_op
    return T


def fog_inscatter(p, sky: Sky, d):
    """Fog colour per ray: sky radiance at the horizon of the ray's azimuth, tinted toward/away from the sun,
    plus sun (and moon) in-scattering with a Henyey-Greenstein lobe."""
    h = d.copy()
    h[:, 1] = np.maximum(h[:, 1], 0.03)
    h /= np.linalg.norm(h, axis=1, keepdims=True)
    amb = sky.radiance(h)
    toward = lin(p["Color"]) / lin(P.DEFAULTS["Color"])
    away = lin(p["Decay"]) / lin(P.DEFAULTS["Decay"])
    out = np.zeros_like(amb)
    for L_dir, E in ((sky.sun, sky.sun_light()), (sky.moon, sky.moon_light())):
        c = d @ L_dir
        w = ((1 + c) * 0.5) ** 4
        tint = away[None] * (1 - w[:, None]) + toward[None] * w[:, None]
        g = 0.55
        hg = (1 - g * g) / (4 * math.pi * np.power(1 + g * g - 2 * g * c, 1.5))
        out += tint * (E[None] * hg[:, None]) * (0.6 + 0.4 * (p["Glare"] > 0))
    toward_s = toward
    return amb * (0.5 * (toward_s + away))[None] + out


# ---------------------------------------------------------------------------------------------
# Render one view.

def render_scene(name, p, w=288, h=162, yaw_offset=35.0, interior=False, lights=(), toon=False, pixel=False,
                 ground=(75, 151, 75)):
    t0 = time.time()
    sky = Sky(p)
    boxes = scene_interior() if interior else scene_outdoor()
    if pixel:
        w, h = w // 2, h // 2
    # camera: faces the sun azimuth rotated by yaw_offset (so the sun can be in frame at dusk)
    key = sky.sun if sky.sun[1] > -0.1 else sky.moon
    az = math.atan2(key[0], key[2]) + math.radians(yaw_offset)
    if interior:
        az = 0.0
    fwd = np.array([math.sin(az), 0.0, math.cos(az)])
    pitch = math.radians(4.0)
    fwd = fwd * math.cos(pitch) + np.array([0, 1, 0]) * math.sin(pitch)
    cam = np.array([0.0, 6.0, 10.0]) - np.array([fwd[0], 0, fwd[2]]) * 34.0
    if interior:
        cam = np.array([0.0, 7.0, -14.0])
    right = np.cross(fwd, [0, 1, 0])
    right /= np.linalg.norm(right)
    up = np.cross(right, fwd)
    fov = math.radians(70)
    xs = (np.arange(w) + 0.5) / w * 2 - 1
    ys = 1 - (np.arange(h) + 0.5) / h * 2
    gx, gy = np.meshgrid(xs * math.tan(fov / 2), ys * math.tan(fov / 2) * h / w)
    d = fwd[None, None] + gx[..., None] * right + gy[..., None] * up
    d = (d / np.linalg.norm(d, axis=-1, keepdims=True)).reshape(-1, 3)
    o = np.broadcast_to(cam, d.shape).copy()
    t, idx, n = trace(o, d, boxes, t_max=6000.0)
    hit = idx != -1
    pos = o + d * t[:, None]

    albedo = np.zeros_like(d)
    rough = np.full(len(d), 0.9)
    neon = np.zeros(len(d))
    ground_col = ground if not interior else (70, 72, 80)
    albedo[idx == -2] = lin(ground_col)
    rough[idx == -2] = 0.95
    for i, (_a, _b, col, r, ne) in enumerate(boxes):
        m = idx == i
        albedo[m] = lin(col)
        rough[m] = r
        neon[m] = ne

    color = np.zeros_like(d)
    # --- sky pixels
    sky_px = ~hit
    if sky_px.any():
        color[sky_px] = sky.radiance(d[sky_px]) * (0.0 if interior else 1.0)
        color[sky_px] += celestial_disks(p, sky, d[sky_px])

    # --- surfaces
    sp = hit
    P_ = pos[sp] + n[sp] * 1e-3
    N = n[sp]
    V = -d[sp]
    alb = albedo[sp]
    lit = np.zeros_like(P_)
    sun_tint = 1.0 + srgb01(p["ColorShift_Top"])
    soft = max(0.27, 3.0 * p["ShadowSoftness"])
    if p["LightingStyle"] == "Soft":
        soft = max(soft, 1.5)
    soft = soft + (20.0 - soft) * p["CloudCover"] ** 2
    for L_dir, E in ((sky.sun, sky.sun_light() * sun_tint), (sky.moon, sky.moon_light())):
        if L_dir[1] <= -0.02 or E.max() <= 0:
            continue
        ndl = np.clip(N @ L_dir, 0, 1)
        vis = np.zeros(len(P_))
        if p["GlobalShadows"]:
            dirs = cone_dirs(L_dir, math.radians(soft), 8)
            for cd in dirs:
                vis += ~occluded(P_, np.broadcast_to(cd, P_.shape), boxes, 2000)
            vis /= len(dirs)
        else:
            vis[:] = 1
        if toon:
            ndl = np.clip((ndl * vis - 0.02) / 0.06, 0, 1)
            vis = np.ones_like(vis)
        diff = alb / math.pi
        H = L_dir[None] + V
        H /= np.linalg.norm(H, axis=1, keepdims=True)
        ndh = np.clip((N * H).sum(1), 0, 1)
        ndv = np.clip((N * V).sum(1), 1e-3, 1)
        a = rough[sp] ** 2
        a2 = a * a
        dd = ndh * ndh * (a2 - 1) + 1
        D = a2 / (math.pi * dd * dd)
        k = (rough[sp] + 1) ** 2 / 8
        G = (ndl / (ndl * (1 - k) + k)) * (ndv / (ndv * (1 - k) + k))
        F = 0.04 + 0.96 * (1 - np.clip((V * H).sum(1), 0, 1)) ** 5
        spec = D * G * F / np.maximum(4 * ndl * ndv, 1e-3)
        lit += (diff + spec[:, None] * (0 if toon else 1)) * E[None] * (ndl * vis)[:, None]

    # sky + ground-bounce ambient through ray-traced sky visibility
    axes = np.array([[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]], float)
    irr = irradiance_table(sky, axes) * p["EnvironmentDiffuseScale"]
    if interior:
        irr *= 0.0  # closed room: no sky
    ai = np.argmax(N @ axes.T, axis=1)
    E_amb = irr[ai]
    if toon:
        E_amb = np.repeat(irr.mean(0, keepdims=True), len(N), 0) * 0.9 + E_amb * 0.1
    vis_sky = sky_visibility(P_, N, boxes)
    lit += alb / math.pi * E_amb * vis_sky[:, None]
    # environment specular (sky in reflection direction)
    R = d[sp] - 2 * (d[sp] * N).sum(1, keepdims=True) * N
    ndv = np.clip((N * V).sum(1), 0, 1)
    f_env = 0.04 + (np.maximum(1 - rough[sp], 0.04) - 0.04)[:, None] * ((1 - ndv) ** 5)[:, None]
    env = sky.radiance(np.where(R[:, 1:2] > 0.01, R, R * np.array([1, -1, 1])))
    if not interior and not toon:
        lit += env * f_env * vis_sky[:, None] * p["EnvironmentSpecularScale"] * (1 - rough[sp])[:, None]

    # point lights (windowed inverse square), with hard shadows
    for (lp, lc, lb, lr) in lights:
        lp = np.array(lp, float)
        Lv = lp[None] - P_
        dist = np.linalg.norm(Lv, axis=1)
        Ld = Lv / dist[:, None]
        ndl = np.clip((N * Ld).sum(1), 0, 1)
        win = np.clip(1 - (dist / lr) ** 4, 0, 1) ** 2
        intensity_cd = 1000.0 * lb  # PointLight.Brightness 1 = 1000 cd (assistant's call)
        e_lux = intensity_cd / np.maximum((dist * STUD_M) ** 2, 0.01) * win
        sh = ~occluded(P_, Ld, boxes, 1e5) | (
            trace(P_, Ld, boxes, ground=False)[0] > dist)
        lit += alb / math.pi * (lin(lc)[None] * (e_lux / SUN_TOA_LUX)[:, None]) * (ndl * sh)[:, None]

    color[sp] = lit

    # --- world fog (height fog) on everything, sky gets the converged integral
    dist = np.where(hit, t, 1e5)
    T = height_fog(p, o, d, dist)
    ins = fog_inscatter(p, sky, d)
    if interior:
        ins *= 0.0
    color = color * T + ins * (1 - T)

    # --- exposure (needs pre-exposure luminance) then exposure-relative terms (Ambient, Neon)
    lum = color @ LUM
    ev100, exposure = auto_exposure(p, lum.reshape(h, w))
    color *= exposure
    amb_k = 0.5  # Ambient 255 white -> pre-exposed radiance 0.5 on a white surface
    surf_amb = lin(p["Ambient"])[None] + lin(p["OutdoorAmbient"])[None] * vis_sky[:, None]
    add = np.zeros_like(color)
    add[sp] = alb * surf_amb * amb_k
    neon_px = neon > 0
    add[neon_px] += albedo[neon_px] * 2.8 * neon[neon_px][:, None]  # Neon: 1.5 EV above display white
    color += add * T
    # Ambient also lights the air (city glow in fog): exposure-relative like surfaces.
    color += (lin(p["Ambient"]) * amb_k)[None] * (1 - T)
    img = color.reshape(h, w, 3)
    img = bloom(img, p["BloomIntensity"], p["BloomSize"] * w / 1920.0)
    img = grade_and_tonemap(p, img, ev100)
    if pixel:
        img = pixelate(img)
    print(f"  {name}: EV100 {ev100:.2f}, {time.time() - t0:.1f}s")
    return img


def sky_visibility(P_, N, boxes, k=12, reach=40.0):
    t_, b_ = basis(N)
    vis = np.zeros(len(P_))
    for i in range(k):
        u1 = (i + 0.5) / k
        u2 = (i * 0.618034) % 1.0
        r = math.sqrt(u1)
        th = 2 * math.pi * u2
        dl = t_ * (r * math.cos(th)) + b_ * (r * math.sin(th)) + N * math.sqrt(1 - u1)
        vis += ~occluded(P_, dl, boxes, reach)
    return vis / k


def celestial_disks(p, sky: Sky, d):
    out = np.zeros_like(d)
    for L_dir, E_toa, size in ((sky.sun, sky.e_sun, p["SunAngularSize"]),
                               (sky.moon, sky.e_moon * 0.12, p["MoonAngularSize"])):
        if L_dir[1] < -0.02:
            continue
        rad = math.radians(size * 0.5)
        c = d @ L_dir
        ang = np.arccos(np.clip(c, -1, 1))
        x = np.clip(ang / rad, 0, 1)
        mu = np.sqrt(np.clip(1 - x * x, 0, 1))
        limb = 1 - 0.6 * (1 - mu)  # simple limb darkening
        inside = (ang < rad).astype(float)
        edge = np.clip((rad - ang) / (rad * 0.08), 0, 1)
        T = sky.light_at_ground(L_dir, 1.0)
        solid = 2 * math.pi * (1 - math.cos(math.radians(0.2666)))
        L_disk = E_toa / solid * T  # luminance of the real sun; disk drawn at the visual size
        out += (inside * edge * limb)[:, None] * L_disk[None] * (1 - p["CloudCover"]) ** 3
    # stars: only visible where the sky is dark; brightness fixed in relative units
    if p["StarCount"] > 0 and sky.sun[1] < 0.05:
        cells = 900
        u = np.arctan2(d[:, 0], d[:, 2])
        v = np.arcsin(np.clip(d[:, 1], -1, 1))
        iu = np.floor((u + math.pi) / (2 * math.pi) * cells)
        iv = np.floor((v + math.pi / 2) / math.pi * cells / 2)
        hsh = np.sin(iu * 12.9898 + iv * 78.233) * 43758.5453
        hsh -= np.floor(hsh)
        density = p["StarCount"] / (cells * cells / 2)
        star = (hsh < density) & (d[:, 1] > 0.02)
        fade = np.clip(-sky.sun[1] * 10, 0, 1)
        out += star[:, None] * (4e-6 * (0.5 + hsh / max(density, 1e-9))[:, None]) * fade
    return out


def auto_exposure(p, lum):
    h, w = lum.shape
    yy, xx = np.mgrid[0:h, 0:w]
    wgt = np.exp(-(((xx / w - 0.5) / 0.35) ** 2 + ((yy / h - 0.5) / 0.35) ** 2))
    l = np.maximum(lum, 1e-12).ravel()
    order = np.argsort(l)
    l_s = l[order]
    w_s = wgt.ravel()[order]
    cw = np.cumsum(w_s) / w_s.sum()
    band = (cw > 0.30) & (cw < 0.95)
    log_avg = (np.log2(l_s[band]) * w_s[band]).sum() / w_s[band].sum()
    l_avg_cd = 2 ** log_avg * SUN_TOA_LUX  # relative -> cd/m^2
    ev100 = math.log2(l_avg_cd * 100 / 12.5)
    ev100 = min(max(ev100, p["ExposureMin"]), p["ExposureMax"])
    s = 1.0 - smoothstep(0.0, 10.0, ev100)
    bias = -2.0 * s * (0.5 + 0.5 * p["NightAdaptation"])
    l_target = 2 ** ev100 * 12.5 / 100 / SUN_TOA_LUX
    exposure = 0.18 / l_target * 2 ** (bias + p["ExposureCompensation"])
    return ev100, exposure


def smoothstep(a, b, x):
    t = min(max((x - a) / (b - a), 0.0), 1.0)
    return t * t * (3 - 2 * t)


def blur(img, sigma):
    if sigma < 0.3:
        return img
    r = int(math.ceil(sigma * 3))
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / sigma) ** 2)
    k /= k.sum()
    pad = np.pad(img, ((r, r), (r, r), (0, 0)), mode="edge")
    tmp = sum(k[i] * pad[:, i:i + img.shape[1] + 0] for i in range(2 * r + 1))
    tmp = tmp[r:-r] if r > 0 else tmp
    pad2 = np.pad(tmp, ((r, r), (0, 0), (0, 0)), mode="edge")
    out = sum(k[i] * pad2[i:i + img.shape[0]] for i in range(2 * r + 1))
    return out


def bloom(img, intensity, size_px):
    if intensity <= 0:
        return img
    acc = np.zeros_like(img)
    wsum = 0.0
    for i, s in enumerate([0.5, 1, 2, 4, 8]):
        wgt = 1.0 / (i + 1)
        acc += blur(img, s * max(size_px, 1.0) / 4.0) * wgt
        wsum += wgt
    return img * (1 - intensity) + (acc / wsum) * intensity


def grade_and_tonemap(p, img, ev100):
    c = img
    l = (c @ LUM)[..., None]
    # Night adaptation: desaturate and shift toward blue as the scene gets darker (scotopic look)
    s = p["NightAdaptation"] * (1.0 - smoothstep(-3.0, 5.0, ev100)) * 0.7
    if s > 0:
        c = c * (1 - s) + l * np.array([0.72, 0.82, 1.0]) * s
    c = c * lin(p["TintColor"])
    l = (c @ LUM)[..., None]
    c = np.maximum(l + (1.0 + p["Saturation"]) * (c - l), 0)
    if p["Contrast"] != 0:
        c = 0.18 * np.power(np.maximum(c, 1e-8) / 0.18, 1.0 + p["Contrast"])
    tm = TM.TONEMAPPERS[p["TonemapperPreset"]]
    c = tm(c * TONEMAP_NORMALISE[p["TonemapperPreset"]])
    if p["Vignette"] > 0:
        h, w = c.shape[:2]
        yy, xx = np.mgrid[0:h, 0:w]
        r2 = ((xx + 0.5) / w - 0.5) ** 2 * (w / h) ** 2 * 0.6 + ((yy + 0.5) / h - 0.5) ** 2 * 1.6
        c = c * (1 - p["Vignette"] * np.clip(r2, 0, 1))[..., None]
    return c


def pixelate(img, levels=6):
    h, w = img.shape[:2]
    bayer = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]) / 16.0 - 0.5
    th = np.tile(bayer, (h // 4 + 1, w // 4 + 1))[:h, :w, None]
    s = TM.srgb_encode(img)
    q = np.round(s * (levels - 1) + th * 0.6) / (levels - 1)
    q = TM.srgb_decode(np.clip(q, 0, 1))
    return q.repeat(2, 0).repeat(2, 1)


# Pre-tonemap scale so 0.18 maps to the same display value as AgX for every curve (assistant's call).
TONEMAP_NORMALISE = {}


def _normalise():
    target = TM.agx(np.array([[0.18, 0.18, 0.18]]))[0, 0]
    for name, f in TM.TONEMAPPERS.items():
        lo, hi = 0.05, 20.0
        for _ in range(60):
            mid = math.sqrt(lo * hi)
            v = f(np.array([[0.18 * mid] * 3]))[0, 0]
            lo, hi = (mid, hi) if v < target else (lo, mid)
        TONEMAP_NORMALISE[name] = math.sqrt(lo * hi)
    TONEMAP_NORMALISE["Clamp"] = 1.0


_normalise()


# ---------------------------------------------------------------------------------------------
# Image helpers.

def save(img, name):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name)
    Image.fromarray(TM.to_png_bytes(img)).save(path, optimize=True)
    print("wrote", os.path.relpath(path), os.path.getsize(path) // 1024, "KB")


def label(img_u8, text, xy=(4, 3)):
    im = Image.fromarray(img_u8)
    dr = ImageDraw.Draw(im)
    x, y = xy
    for dx, dy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        dr.text((x + dx, y + dy), text, fill=(0, 0, 0))
    dr.text((x, y), text, fill=(235, 235, 235))
    return np.array(im)


def montage(tiles, cols):
    h, w = tiles[0].shape[:2]
    rows = (len(tiles) + cols - 1) // cols
    out = np.full((rows * h + (rows - 1) * 2, cols * w + (cols - 1) * 2, 3), 20, np.uint8)
    for i, t in enumerate(tiles):
        r, c = divmod(i, cols)
        out[r * (h + 2):r * (h + 2) + h, c * (w + 2):c * (w + 2) + w] = t
    return out


# ---------------------------------------------------------------------------------------------
# Groups.

def group_sky():
    """Sky looking toward the sun at several elevations, plus an upward fisheye at noon and dusk."""
    tiles = []
    for elev in [60, 25, 10, 4, 1, -2, -5, -9]:
        p = dict(P.DEFAULTS)
        sky = Sky(p, sun_elevation=elev)
        w, h = 240, 135
        az = math.atan2(sky.sun[0], sky.sun[2])
        fwd = np.array([math.sin(az), 0.0, math.cos(az)])
        pitch = math.radians(22)
        fwd = fwd * math.cos(pitch) + np.array([0, 1, 0]) * math.sin(pitch)
        right = np.cross(fwd, [0, 1, 0])
        right /= np.linalg.norm(right)
        up = np.cross(right, fwd)
        fov = math.radians(100)
        xs = (np.arange(w) + 0.5) / w * 2 - 1
        ys = 1 - (np.arange(h) + 0.5) / h * 2
        gx, gy = np.meshgrid(xs * math.tan(fov / 2), ys * math.tan(fov / 2) * h / w)
        d = fwd + gx[..., None] * right + gy[..., None] * up
        d = (d / np.linalg.norm(d, axis=-1, keepdims=True)).reshape(-1, 3)
        col = sky.radiance(d) + celestial_disks(p, sky, d)
        below = d[:, 1] < 0
        # ground below the horizon: lit grey plane with the same fog as the engine
        e_h = horizontal_illuminance(sky)
        col[below] = 0.18 * e_h / math.pi
        o = np.zeros_like(d)
        o[:, 1] = 6.0
        dist = np.where(below, 6.0 / np.maximum(-d[:, 1], 1e-4), 1e5)
        T = height_fog(p, o, d, dist)
        col = col * T + fog_inscatter(p, sky, d) * (1 - T)
        ev, ex = auto_exposure(p, (col @ LUM).reshape(h, w))
        img = bloom((col * ex).reshape(h, w, 3), p["BloomIntensity"], 2.0)
        img = grade_and_tonemap(p, img, ev)
        tiles.append(label(TM.to_png_bytes(img), f"sun {elev:+d} deg  EV100 {ev:.1f}"))
        print(f"  sky elev {elev}: EV100 {ev:.2f}")
    m = montage(tiles, 4)
    Image.fromarray(m).save(os.path.join(OUT, "sky_elevations.png"), optimize=True)
    print("wrote sky_elevations.png", os.path.getsize(os.path.join(OUT, "sky_elevations.png")) // 1024, "KB")


def group_curves():
    names = ["AgX", "AgX Punchy", "PBR Neutral", "ACES (Hill)", "ACES (Narkowicz)", "Clamp"]
    cols = [(230, 230, 230), (250, 170, 60), (90, 200, 120), (90, 150, 250), (200, 110, 220), (150, 150, 150)]
    W, H = 640, 360
    im = Image.new("RGB", (W, H), (24, 25, 28))
    dr = ImageDraw.Draw(im)
    x0, x1, y0, y1 = 48, W - 16, H - 36, 16
    ev_lo, ev_hi = -8.0, 8.0
    for ev in range(int(ev_lo), int(ev_hi) + 1):
        x = x0 + (ev - ev_lo) / (ev_hi - ev_lo) * (x1 - x0)
        dr.line([(x, y0), (x, y1)], fill=(45, 46, 52) if ev else (90, 90, 96))
        if ev % 2 == 0:
            dr.text((x - 6, y0 + 6), f"{ev:+d}", fill=(160, 160, 165))
    for v in [0.0, 0.25, 0.5, 0.75, 1.0]:
        y = y0 + v * (y1 - y0)
        dr.line([(x0, y), (x1, y)], fill=(45, 46, 52))
        dr.text((8, y - 6), f"{v:.2f}", fill=(160, 160, 165))
    evs = np.linspace(ev_lo, ev_hi, 400)
    xin = 0.18 * 2 ** evs
    for name, col in zip(names, cols):
        f = TM.TONEMAPPERS[name]
        k = TONEMAP_NORMALISE[name]
        y = TM.srgb_encode(f(np.repeat(xin[:, None], 3, 1) * k)[:, 1])
        pts = [(x0 + (e - ev_lo) / (ev_hi - ev_lo) * (x1 - x0), y0 + v * (y1 - y0)) for e, v in zip(evs, y)]
        dr.line(pts, fill=col, width=2)
    for i, (name, col) in enumerate(zip(names, cols)):
        dr.rectangle([x0 + 8, y1 + 4 + i * 14, x0 + 20, y1 + 12 + i * 14], fill=col)
        dr.text((x0 + 26, y1 + 2 + i * 14), name, fill=(220, 220, 225))
    dr.text((x1 - 300, y0 - 16), "x: EV relative to mid-grey 0.18; y: sRGB out", fill=(160, 160, 165))
    im.save(os.path.join(OUT, "tonemap_curves.png"), optimize=True)
    print("wrote tonemap_curves.png")

    # Hue sweep: rows = hues of saturated Part colours, columns = exposure -3..+7 EV, one block per curve
    part_cols = [(196, 40, 28), (245, 205, 48), (75, 151, 75), (0, 255, 0), (13, 105, 172), (0, 255, 255),
                 (170, 0, 170), (255, 102, 204), (242, 243, 243)]
    evs = np.arange(-3, 8)
    cw, chh = 22, 14
    blocks = []
    for name in names:
        f = TM.TONEMAPPERS[name]
        k = TONEMAP_NORMALISE[name]
        rows = []
        for c in part_cols:
            base = lin(c)
            vals = np.array([base * 0.18 / 0.2 * 2.0 ** e for e in evs]) * k
            out = TM.to_png_bytes(f(vals))
            rows.append(np.repeat(np.repeat(out[None], chh, 0), cw, 1))
        blk = np.concatenate(rows, 0)
        blk = np.concatenate([np.full((14, blk.shape[1], 3), 24, np.uint8), blk], 0)
        blocks.append(label(blk, name, (2, 1)))
    gap = np.full((blocks[0].shape[0], 4, 3), 24, np.uint8)
    row1 = np.concatenate([blocks[0], gap, blocks[1], gap, blocks[2]], 1)
    row2 = np.concatenate([blocks[3], gap, blocks[4], gap, blocks[5]], 1)
    sheet = np.concatenate([row1, np.full((4, row1.shape[1], 3), 24, np.uint8), row2], 0)
    Image.fromarray(sheet).save(os.path.join(OUT, "tonemap_hues.png"), optimize=True)
    print("wrote tonemap_hues.png (columns: -3..+7 EV, Part colours lit to mid-grey at 0)")


SCENE_SETUP = {
    "Clear Day": dict(yaw_offset=150.0),
    "Golden Hour": dict(yaw_offset=28.0),
    "Overcast": dict(yaw_offset=120.0),
    "Night City": dict(yaw_offset=170.0, ground=(90, 90, 96)),
    "Moonlit": dict(yaw_offset=165.0),
    "Horror Fog": dict(yaw_offset=20.0),
    "Sci-fi Interior": dict(interior=True),
    "Cartoon Bright": dict(yaw_offset=140.0, toon=True, ground=(95, 165, 80)),
    "Pixel Dusk": dict(yaw_offset=25.0, toon=True, pixel=True),
    "Underwater": dict(yaw_offset=150.0, ground=(194, 178, 128)),
    "Desert Noon": dict(yaw_offset=160.0, ground=(204, 170, 120)),
    "Alien Dusk": dict(yaw_offset=25.0, ground=(150, 95, 70)),
}


def group_presets(only=None):
    tiles = []
    for name, p in P.PRESETS.items():
        if only and name not in only:
            continue
        setup = dict(SCENE_SETUP.get(name, {}))
        img = render_scene(name, p, lights=P.SCENE_LIGHTS.get(name, ()), **setup)
        tiles.append(label(TM.to_png_bytes(img), name))
    m = montage(tiles, 3)
    fname = "presets.png" if not only else "presets_partial.png"
    Image.fromarray(m).save(os.path.join(OUT, fname), optimize=True)
    print("wrote", fname, os.path.getsize(os.path.join(OUT, fname)) // 1024, "KB")


def group_table():
    """Numbers for the M1 hook: sun colour and gradient-sky colours versus sun elevation, default atmosphere."""
    p = dict(P.DEFAULTS)
    rows = []
    for elev in [90, 60, 45, 30, 20, 10, 5, 2, 0, -2, -4, -6]:
        sky = Sky(p, sun_elevation=elev)
        sun = sky.sun
        az = math.atan2(sun[0], sun[2])
        zen = sky.radiance(np.array([[0.0, 1.0, 0.0]]))[0]
        dirs = np.array([[math.sin(az + a), 0.02, math.cos(az + a)] for a in np.linspace(0, 2 * math.pi, 16,
                                                                                            endpoint=False)])
        hor = sky.radiance(dirs / np.linalg.norm(dirs, axis=1, keepdims=True)).mean(0)
        sun_e = sky.sun_light()
        e_sky = sky_horizontal(sky)
        e_sun_h = sun_e * max(sun[1], 0)
        rows.append((elev, sun_e, zen, hor, e_sky, e_sun_h))
    lines = ["| Sun elev | Sun illuminance at ground (rel. RGB) | Zenith radiance | Horizon radiance (avg) | "
             "Sky irradiance on ground | Sky / (sky+sun) |", "|---|---|---|---|---|---|"]
    for elev, s, z, hrz, es, eh in rows:
        frac = (es @ LUM) / max((es + eh) @ LUM, 1e-12)
        f3 = lambda v: "(" + ", ".join(f"{x:.3g}" for x in v) + ")"  # noqa: E731
        lines.append(f"| {elev:+d} | {f3(s)} | {f3(z)} | {f3(hrz)} | {f3(es)} | {frac:.2f} |")
    txt = "\n".join(lines)
    with open(os.path.join(OUT, "sky_table.md"), "w", encoding="utf-8") as fh:
        fh.write("# Sky reference values (default atmosphere, relative units: sun at top of atmosphere = 1)\n\n")
        fh.write("Generated by `docs/research/lighting-shaders/reference/render.py table`.\n\n")
        fh.write(txt + "\n")
    print(txt)


def group_m1():
    """Table for the Task 3 hook: Sky struct fields (renderer units) and a CPU exposure per sun elevation.

    Renderer units: scaled so the 45-degree sun matches today's Sky::default() sun_color luminance (3.33).
    Exposure: key / estimated scene luminance (half ground, half horizon), with the night bias curve,
    normalised to 1.0 at 45 degrees so today's look is unchanged at mid-afternoon.
    """
    p = dict(P.DEFAULTS)
    today_sun_lum = float(np.array([3.5, 3.3, 3.05]) @ LUM)
    rows = []
    k = None
    ref_exposure = None
    elevs = [90, 60, 45, 30, 20, 15, 10, 6, 3, 1, 0, -2, -4, -6, -9, -12, -18]
    data = []
    for elev in elevs:
        sky = Sky(p, sun_elevation=elev)
        sun = sky.sun_light() + sky.moon_light() * (sky.sun[1] < 0)
        az = math.atan2(sky.sun[0], sky.sun[2])
        zen = sky.radiance(np.array([[0.0, 1.0, 0.0]]))[0]
        if elev == 90:  # straight up is the sun's glow; sample 10 degrees off zenith instead
            zen = sky.radiance(np.array([[math.sin(math.radians(10)), math.cos(math.radians(10)), 0.0]]))[0]
        dirs = np.array([[math.sin(az + a), 0.05, math.cos(az + a)] for a in np.linspace(0, 2 * math.pi, 24,
                                                                                            endpoint=False)])
        hor = sky.radiance(dirs / np.linalg.norm(dirs, axis=1, keepdims=True)).mean(0)
        e_h = horizontal_illuminance(sky)
        ground = 0.2 * e_h / math.pi
        l_scene = float((0.5 * ground + 0.5 * hor) @ LUM)
        data.append((elev, sun, zen, hor, ground, l_scene))
        if elev == 45:
            k = today_sun_lum / float(sun @ LUM)
    for elev, sun, zen, hor, ground, l_scene in data:
        l_cd = l_scene * SUN_TOA_LUX
        ev = min(max(math.log2(max(l_cd, 1e-9) * 8.0), p["ExposureMin"]), p["ExposureMax"])
        s = 1.0 - smoothstep(0.0, 10.0, ev)
        bias = -2.0 * s
        exposure = 0.18 / (2 ** ev * 0.125 / SUN_TOA_LUX) * 2 ** bias
        if elev == 45:
            ref_exposure = exposure
        rows.append((elev, sun * k, zen * k, hor * k, ground * k, ev, exposure))
    f3 = lambda v: "[" + ", ".join(f"{x:.3g}" for x in v) + "]"  # noqa: E731
    lines = ["| Sun elevation | sun_color | zenith | horizon | ground | metered EV100 | exposure |",
             "|---|---|---|---|---|---|---|"]
    for elev, s, z, h, g, ev, ex in rows:
        lines.append(f"| {elev:+d} | {f3(s)} | {f3(z)} | {f3(h)} | {f3(g)} | {ev:.1f} | {ex / ref_exposure:.3g} |")
    txt = "\n".join(lines)
    with open(os.path.join(OUT, "m1_sky_table.md"), "w", encoding="utf-8") as fh:
        fh.write("# Sky struct values per sun elevation (Task 3 hook)\n\n")
        fh.write("Generated by `python docs/research/lighting-shaders/reference/render.py m1`. Default atmosphere, "
                 "renderer units (45 deg sun = today's `sun_color` luminance). Below the horizon `sun_color` is "
                 "the moon (full, MoonBrightness 1). Interpolate linearly in elevation; the exposure column is "
                 "a CPU stand-in for histogram metering, 1.0 at 45 deg.\n\n")
        fh.write(txt + "\n")
    print(txt)


def group_tonemap_scenes():
    """Same scenes through four curves, to pick the default."""
    names = ["Clear Day", "Golden Hour", "Night City"]
    curves = ["AgX", "AgX Punchy", "PBR Neutral", "ACES (Hill)"]
    tiles = []
    for c in curves:
        for n in names:
            p = dict(P.PRESETS[n])
            p["TonemapperPreset"] = c
            img = render_scene(n, p, w=240, h=135, lights=P.SCENE_LIGHTS.get(n, ()), **SCENE_SETUP.get(n, {}))
            tiles.append(label(TM.to_png_bytes(img), f"{n} / {c}"))
    m = montage(tiles, 3)
    Image.fromarray(m).save(os.path.join(OUT, "tonemap_scenes.png"), optimize=True)
    print("wrote tonemap_scenes.png")


if __name__ == "__main__":
    groups = sys.argv[1:] or ["curves", "sky", "table", "m1", "presets", "tmscenes"]
    for g in groups:
        print("==", g)
        if g == "sky":
            group_sky()
        elif g == "curves":
            group_curves()
        elif g == "presets":
            group_presets()
        elif g == "table":
            group_table()
        elif g == "tmscenes":
            group_tonemap_scenes()
        elif g == "m1":
            group_m1()
        elif g.startswith("preset:"):
            group_presets(g.split(":", 1)[1].split(","))
