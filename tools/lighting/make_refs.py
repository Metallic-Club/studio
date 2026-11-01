"""Numeric references for the engine's lighting shaders (#155), from the numpy reference renderer
(docs/research/lighting-shaders/reference/, issue #37).

    python -I tools/lighting/make_refs.py

Writes crates/metallic_render/tests/data/lighting_refs.json; `tests/lighting_refs.rs` checks the
engine against it. Needs numpy only (not Pillow): it imports atmosphere.py and tonemap.py and
mirrors the few lines of render.py it needs (render.py imports Pillow at module level).

Contents:
- sky: sun colour, zenith and horizon radiance per sun elevation in renderer units, sampled exactly
  as `render.py m1` sampled them for docs/agents/37-lighting/m1_sky_table.md (the engine's table).
  The table elevations plus elevations between them, where the engine interpolates.
- fog: optical depth of exponential height fog between two points (render.py `height_fog`).
- curves: the six tone curves of tonemap.py on a grey ramp and on saturated colours.
"""
from __future__ import annotations

import json
import math
import os
import sys

import numpy as np

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "docs", "research", "lighting-shaders", "reference"))
import atmosphere as A  # noqa: E402
import tonemap as TM  # noqa: E402

OUT = os.path.join(ROOT, "crates", "metallic_render", "tests", "data", "lighting_refs.json")
LUM = np.array([0.2126, 0.7152, 0.0722])
SUN_TOA_LUX = 120000.0
MOON_LUX = 0.25

# Elevations of the engine's table (m1_sky_table.md) and between them.
TABLE = [90, 60, 45, 30, 20, 15, 10, 6, 3, 1, 0, -2, -4, -6, -9, -12, -18]
BETWEEN = [75, 52, 37, 25, 17.5, 12.5, 8, 4.5, 2, 0.5, -1, -3, -5, -7.5, -10.5, -15]


# ---- Sky (mirrors render.py: atmosphere_for, Sky.radiance, Sky.sun_light, group_m1) ----------

def default_atmosphere() -> A.Atmosphere:
    """`atmosphere_for(DEFAULTS)`: Haze 1, Glare 2, default Color and AirColor (tints of 1)."""
    atm = A.Atmosphere()
    mie_scale = 1.0 + 0.8 * 1.0
    atm.mie_scattering = atm.mie_scattering * mie_scale
    atm.mie_extinction = np.maximum(atm.mie_extinction * mie_scale, atm.mie_scattering)
    atm.mie_g = min(0.76 + 0.019 * 2.0, 0.95)
    return atm


def to_local(d):
    return np.stack([d[..., 0], d[..., 2], d[..., 1]], -1)


class Sky:
    """Sun (west-facing, at an exact elevation) and moon opposite, as render.py's Sky."""

    def __init__(self, atm, lut_t, lut_ms, elevation):
        self.atm, self.lut_t = atm, lut_t
        e = math.radians(elevation)
        self.sun = np.array([-math.cos(e), math.sin(e), 0.0])
        self.moon = -self.sun
        self.r = atm.bottom_radius + 0.0002
        self.e_moon = MOON_LUX / SUN_TOA_LUX
        self.lights = []
        for d, energy in ((self.sun, 1.0), (self.moon, self.e_moon)):
            elev = math.degrees(math.asin(max(-1.0, min(1.0, d[1]))))
            lut, _ = A.build_skyview(atm, lut_t, lut_ms, elev)
            self.lights.append((lut, d, energy))

    def radiance(self, dirs):
        out = np.zeros(dirs.shape)
        for lut, d, energy in self.lights:
            out += energy * A.sample_skyview(self.atm, lut, self.r, to_local(dirs), to_local(d))
        return out

    def light(self, d, energy):
        elev = math.degrees(math.asin(max(-1.0, min(1.0, d[1]))))
        return energy * A.sun_transmittance(self.atm, self.lut_t, elev)

    def sun_colour(self):
        """The light that lights the scene: the sun, or below the horizon the moon."""
        s = self.light(self.sun, 1.0)
        if self.sun[1] < 0:
            s = s + self.light(self.moon, self.e_moon)
        return s


def sky_refs():
    atm = default_atmosphere()
    lut_t = A.build_transmittance(atm)
    lut_ms = A.build_multiscatter(atm, lut_t)
    rows = []
    for elev in sorted(TABLE + BETWEEN, reverse=True):
        sky = Sky(atm, lut_t, lut_ms, elev)
        az = math.atan2(sky.sun[0], sky.sun[2])
        if elev == 90:  # straight up is the sun's glow: render.py samples 10 degrees off zenith
            up = np.array([[math.sin(math.radians(10)), math.cos(math.radians(10)), 0.0]])
        else:
            up = np.array([[0.0, 1.0, 0.0]])
        zen = sky.radiance(up)[0]
        ring = np.array([[math.sin(az + a), 0.05, math.cos(az + a)]
                         for a in np.linspace(0, 2 * math.pi, 24, endpoint=False)])
        hor = sky.radiance(ring / np.linalg.norm(ring, axis=1, keepdims=True)).mean(0)
        rows.append({"elevation": float(elev), "table": elev in TABLE,
                     "sun": sky.sun_colour(), "zenith": zen, "horizon": hor})
        print(f"sky {elev:+6.1f}", file=sys.stderr)
    # Renderer units: the 45-degree sun has the luminance of the renderer's old default sun.
    today_sun_lum = float(np.array([3.5, 3.3, 3.05]) @ LUM)
    k = today_sun_lum / float(next(r["sun"] for r in rows if r["elevation"] == 45) @ LUM)
    for r in rows:
        for key in ("sun", "zenith", "horizon"):
            r[key] = [float(v) for v in r[key] * k]
    return rows


# ---- Height fog (mirrors render.py `height_fog`; absorption 0, offset 0) ---------------------

def fog_tau(sigma0, falloff, fog_height, p0, p1):
    o = np.asarray(p0, dtype=np.float64)
    v = np.asarray(p1, dtype=np.float64) - o
    dist = float(np.linalg.norm(v))
    d = v / dist
    base = sigma0 * math.exp(-falloff * (o[1] - fog_height)) if falloff > 0 else sigma0
    if falloff > 0:
        k = falloff * d[1] * dist
        integ = (1.0 - math.exp(-k)) / (falloff * d[1]) if abs(k) > 1e-4 else dist
    else:
        integ = dist
    return base * integ


def fog_refs():
    cases = []
    # (sigma0, falloff, fog height, eye, point): level, climbing, descending, long, near-flat.
    setups = [
        (0.00108, 0.02, 0.0),  # the default Atmosphere in the engine (Density 0.3, Haze 1)
        (0.004, 0.05, 10.0),
        (0.02, 0.008, -5.0),
    ]
    paths = [
        ((0.0, 5.0, 0.0), (0.0, 5.0, -200.0)),
        ((0.0, 2.0, 0.0), (50.0, 80.0, -300.0)),
        ((0.0, 120.0, 0.0), (0.0, 1.0, -400.0)),
        ((10.0, 30.0, 5.0), (-900.0, 60.0, 700.0)),
        ((0.0, 3.0, 0.0), (0.0, 3.04, -500.0)),
    ]
    for s0, f, h in setups:
        for p0, p1 in paths:
            cases.append({"sigma0": s0, "falloff": f, "fog_height": h, "p0": list(p0), "p1": list(p1),
                          "tau": fog_tau(s0, f, h, p0, p1)})
    return cases


# ---- Tone curves (tonemap.py), engine order (ToneCurve::index) --------------------------------

def curve_refs():
    curves = [("Filmic", TM.aces_narkowicz), ("Agx", TM.agx), ("AgxPunchy", lambda c: TM.agx(c, "punchy")),
              ("Neutral", TM.pbr_neutral), ("Aces", TM.aces_hill), ("Clamp", TM.clamp_curve)]
    inputs = [[v, v, v] for v in (2.0 ** np.linspace(-10, 8, 37)).tolist()]
    for v in (0.02, 0.18, 1.0, 6.0):
        for tint in ((1.0, 0.3, 0.05), (0.05, 0.2, 1.0), (0.2, 1.0, 0.1), (1.0, 0.0, 0.8)):
            inputs.append([v * t for t in tint])
    x = np.array(inputs, dtype=np.float64)
    out = {name: np.asarray(fn(x), dtype=np.float64).tolist() for name, fn in curves}
    return {"inputs": inputs, "outputs": out}


def main():
    refs = {
        "source": "tools/lighting/make_refs.py from docs/research/lighting-shaders/reference (numpy "
                  + np.__version__ + ")",
        "sky": sky_refs(),
        "fog": fog_refs(),
        "curves": curve_refs(),
    }
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(refs, fh, indent=1)
        fh.write("\n")
    print("wrote", os.path.relpath(OUT, ROOT))


if __name__ == "__main__":
    main()
