"""CPU reference of the Hillaire 2020 sky model, written to match the WGSL in ../atmosphere_*.wgsl.

Units: kilometres, and luminance per unit of sun illuminance (the LUTs are "transfer" tables;
multiply by the sun illuminance at run time). Same Earth constants as the MIT reference code
(sebh/UnrealEngineSkyAtmosphere, SetupEarthAtmosphere).

Run from the repo root:  python docs/research/lighting-shaders/reference/atmosphere.py
Needs numpy and Pillow only.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np

PI = math.pi


@dataclass
class Atmosphere:
    bottom_radius: float = 6360.0
    top_radius: float = 6460.0
    rayleigh_scattering: np.ndarray = field(default_factory=lambda: np.array([0.005802, 0.013558, 0.033100]))
    rayleigh_exp_scale: float = -1.0 / 8.0
    mie_scattering: np.ndarray = field(default_factory=lambda: np.array([0.003996] * 3))
    mie_extinction: np.ndarray = field(default_factory=lambda: np.array([0.004440] * 3))
    mie_exp_scale: float = -1.0 / 1.2
    mie_g: float = 0.8
    # Ozone tent: 0 at 10 km, 1 at 25 km, 0 at 40 km.
    ozone_absorption: np.ndarray = field(default_factory=lambda: np.array([0.000650, 0.001881, 0.000085]))
    ground_albedo: np.ndarray = field(default_factory=lambda: np.array([0.3, 0.3, 0.3]))

    def medium(self, h: np.ndarray):
        """h: altitude in km, any shape. Returns (scattering_ray, scattering_mie, extinction), each [..., 3]."""
        h = np.maximum(h, 0.0)[..., None]
        d_ray = np.exp(self.rayleigh_exp_scale * h)
        d_mie = np.exp(self.mie_exp_scale * h)
        d_ozo = np.maximum(0.0, 1.0 - np.abs(h - 25.0) / 15.0)
        s_ray = d_ray * self.rayleigh_scattering
        s_mie = d_mie * self.mie_scattering
        ext = s_ray + d_mie * self.mie_extinction + d_ozo * self.ozone_absorption
        return s_ray, s_mie, ext


def ray_sphere(r: np.ndarray, mu: np.ndarray, radius: float):
    """Distance along a ray starting at height r (from planet centre) with zenith cosine mu to a sphere.
    Returns (t_near, t_far); NaN where there is no hit."""
    disc = r * r * (mu * mu - 1.0) + radius * radius
    sq = np.sqrt(np.where(disc >= 0.0, disc, np.nan))
    return -r * mu - sq, -r * mu + sq


def phase_rayleigh(c):
    return 3.0 / (16.0 * PI) * (1.0 + c * c)


def phase_cornette_shanks(c, g):
    k = 3.0 / (8.0 * PI) * (1.0 - g * g) / (2.0 + g * g)
    return k * (1.0 + c * c) / np.power(1.0 + g * g - 2.0 * g * c, 1.5)


# ---- LUT parameterisations (same as the WGSL) -------------------------------------------

def from_unit_to_sub_uv(u, res):
    return (u + 0.5 / res) * (res / (res + 1.0))


def from_sub_uv_to_unit(u, res):
    return (u - 0.5 / res) * (res / (res - 1.0))


def transmittance_uv_to_r_mu(atm: Atmosphere, u, v):
    H = math.sqrt(atm.top_radius ** 2 - atm.bottom_radius ** 2)
    rho = H * v
    r = np.sqrt(rho * rho + atm.bottom_radius ** 2)
    d_min = atm.top_radius - r
    d_max = rho + H
    d = d_min + u * (d_max - d_min)
    mu = np.where(d == 0.0, 1.0, (H * H - rho * rho - d * d) / (2.0 * r * np.maximum(d, 1e-9)))
    return r, np.clip(mu, -1.0, 1.0)


def transmittance_r_mu_to_uv(atm: Atmosphere, r, mu):
    H = math.sqrt(atm.top_radius ** 2 - atm.bottom_radius ** 2)
    rho = np.sqrt(np.maximum(r * r - atm.bottom_radius ** 2, 0.0))
    disc = r * r * (mu * mu - 1.0) + atm.top_radius ** 2
    d = np.maximum(0.0, -r * mu + np.sqrt(np.maximum(disc, 0.0)))
    d_min = atm.top_radius - r
    d_max = rho + H
    u = (d - d_min) / np.maximum(d_max - d_min, 1e-9)
    v = rho / H
    return u, v


def sample_bilinear(lut: np.ndarray, u, v):
    """lut [H, W, C]; u, v in [0,1] over texel centres like a GPU linear clamp sampler."""
    h, w = lut.shape[:2]
    x = np.clip(u * w - 0.5, 0.0, w - 1.0)
    y = np.clip(v * h - 0.5, 0.0, h - 1.0)
    x0 = np.floor(x).astype(int)
    y0 = np.floor(y).astype(int)
    x1 = np.minimum(x0 + 1, w - 1)
    y1 = np.minimum(y0 + 1, h - 1)
    fx = (x - x0)[..., None]
    fy = (y - y0)[..., None]
    a = lut[y0, x0] * (1 - fx) + lut[y0, x1] * fx
    b = lut[y1, x0] * (1 - fx) + lut[y1, x1] * fx
    return a * (1 - fy) + b * fy


# ---- 1. transmittance LUT 256x64, 40 steps -------------------------------------------

def build_transmittance(atm: Atmosphere, w=256, h=64, steps=40):
    u = (np.arange(w) + 0.5) / w
    v = (np.arange(h) + 0.5) / h
    uu, vv = np.meshgrid(u, v)
    r, mu = transmittance_uv_to_r_mu(atm, uu, vv)
    _, t_top = ray_sphere(r, mu, atm.top_radius)
    dt = t_top / steps
    optical = np.zeros(r.shape + (3,))
    for i in range(steps):
        t = (i + 0.5) * dt
        p_r = np.sqrt(r * r + t * t + 2.0 * r * mu * t)
        _, _, ext = atm.medium(p_r - atm.bottom_radius)
        optical += ext * dt[..., None]
    return np.exp(-optical)


def transmittance_to_top(atm, lut_t, r, mu):
    u, v = transmittance_r_mu_to_uv(atm, r, mu)
    return sample_bilinear(lut_t, u, v)


# ---- shared single-scattering integrator ---------------------------------------------

def integrate(atm: Atmosphere, lut_t, lut_ms, r0, view_dir, sun_dir, steps, mie_ray_phase=True,
              ground=True, isotropic=False):
    """Ray march from radius r0 at the north pole (+z up) along view_dir [...,3].
    Returns (L [...,3], f_ms [...,3], transmittance [...,3]).
    sun_dir: [3] or broadcastable. lut_ms may be None (no multiple scattering)."""
    shape = view_dir.shape[:-1]
    pos0 = np.zeros(shape + (3,))
    pos0[..., 2] = r0
    mu = view_dir[..., 2]
    r0a = np.full(shape, r0)
    t_g0, _ = ray_sphere(r0a, mu, atm.bottom_radius)
    _, t_top = ray_sphere(r0a, mu, atm.top_radius)
    hit_ground = np.isfinite(t_g0) & (t_g0 > 0.0)
    t_max = np.where(hit_ground, t_g0, np.nan_to_num(t_top, nan=0.0))
    t_max = np.maximum(t_max, 0.0)
    dt = t_max / steps
    L = np.zeros(shape + (3,))
    f_ms = np.zeros(shape + (3,))
    T = np.ones(shape + (3,))
    cos_theta = np.sum(view_dir * sun_dir, axis=-1)
    if isotropic:
        ph_ray = np.full(shape, 1.0 / (4.0 * PI))
        ph_mie = ph_ray
    else:
        ph_ray = phase_rayleigh(cos_theta)
        ph_mie = phase_cornette_shanks(cos_theta, atm.mie_g)
    for i in range(steps):
        t = (i + 0.3) * dt  # 0.3 offset, as in the reference code
        p = pos0 + view_dir * t[..., None]
        pr = np.linalg.norm(p, axis=-1)
        up = p / pr[..., None]
        s_ray, s_mie, ext = atm.medium(pr - atm.bottom_radius)
        sun_mu = np.sum(up * sun_dir, axis=-1)
        t_sun = transmittance_to_top(atm, lut_t, pr, sun_mu)
        # Earth shadow: is the sun below the planet horizon from this sample?
        t_e, _ = ray_sphere(pr, sun_mu, atm.bottom_radius)
        earth_shadow = np.where(np.isfinite(t_e) & (t_e > 0.0), 0.0, 1.0)[..., None]
        ms = 0.0
        if lut_ms is not None:
            ms = sample_ms(lut_ms, sun_mu, pr, atm)
        scat = s_ray + s_mie
        phase_scat = s_ray * ph_ray[..., None] + s_mie * ph_mie[..., None]
        S = earth_shadow * t_sun * phase_scat + ms * scat
        step_T = np.exp(-ext * dt[..., None])
        # Energy-conserving analytic integration over the segment (Hillaire 2015).
        safe_ext = np.maximum(ext, 1e-9)
        Sint = (S - S * step_T) / safe_ext
        L += T * Sint
        MSint = (scat - scat * step_T) / safe_ext
        f_ms += T * MSint
        T *= step_T
    if ground:
        # Lambertian ground lit by the sun, only where the ray hits the planet.
        pg = pos0 + view_dir * t_max[..., None]
        pgr = np.linalg.norm(pg, axis=-1)
        upg = pg / pgr[..., None]
        ndl = np.clip(np.sum(upg * sun_dir, axis=-1), 0, 1)
        t_sun_g = transmittance_to_top(atm, lut_t, pgr, np.sum(upg * sun_dir, axis=-1))
        L += np.where(hit_ground[..., None], T * t_sun_g * ndl[..., None] * atm.ground_albedo / PI, 0.0)
    return L, f_ms, T


# ---- 2. multiple scattering LUT 32x32, 64 directions x 20 steps ----------------------------

def build_multiscatter(atm: Atmosphere, lut_t, res=32, sqrt_dirs=8, steps=20):
    out = np.zeros((res, res, 3))
    # 64 uniform sphere directions, same layout as the WGSL (8x8 grid).
    ij = (np.arange(sqrt_dirs) + 0.5) / sqrt_dirs
    a, b = np.meshgrid(ij, ij)
    theta = 2 * PI * a.ravel()
    phi = np.arccos(1.0 - 2.0 * b.ravel())
    dirs = np.stack([np.cos(theta) * np.sin(phi), np.sin(theta) * np.sin(phi), np.cos(phi)], -1)
    n = dirs.shape[0]
    for y in range(res):
        for x in range(res):
            u = from_sub_uv_to_unit((x + 0.5) / res, res)
            v = from_sub_uv_to_unit((y + 0.5) / res, res)
            cos_sun = u * 2.0 - 1.0
            sun = np.array([0.0, math.sqrt(max(0.0, 1.0 - cos_sun * cos_sun)), cos_sun])
            r = atm.bottom_radius + np.clip(v + 0.01, 0, 1) * (atm.top_radius - atm.bottom_radius - 0.01)
            L, fms, _ = integrate(atm, lut_t, None, r, dirs, sun, steps, ground=True, isotropic=True)
            L2 = L.sum(0) * (4 * PI / n) / (4 * PI)
            f = fms.sum(0) * (4 * PI / n) / (4 * PI)
            out[y, x] = L2 / (1.0 - f)
    return out


def sample_ms(lut_ms, sun_mu, r, atm):
    u = from_unit_to_sub_uv(np.clip(sun_mu * 0.5 + 0.5, 0, 1), lut_ms.shape[1])
    v = from_unit_to_sub_uv(np.clip((r - atm.bottom_radius) / (atm.top_radius - atm.bottom_radius), 0, 1),
                            lut_ms.shape[0])
    return sample_bilinear(lut_ms, u, v)


# ---- 3. sky-view LUT 192x108, 30 steps ---------------------------------------------------

def skyview_uv_to_dir(atm, r, u, v):
    """Returns view directions in a local frame where +z is up and the sun lies in the x-z plane (azimuth 0)."""
    v_horizon = math.sqrt(max(r * r - atm.bottom_radius ** 2, 0.0))
    cos_beta = v_horizon / r
    beta = math.acos(cos_beta)
    zenith_horizon = PI - beta
    upper = v < 0.5
    c_up = 1.0 - 2.0 * v
    c_up = 1.0 - c_up * c_up
    c_lo = v * 2.0 - 1.0
    c_lo = c_lo * c_lo
    zen = np.where(upper, zenith_horizon * c_up, zenith_horizon + beta * c_lo)
    azi = (u * u) * PI  # nonlinear: more texels toward the sun (u=0)
    cz = np.cos(zen)
    sz = np.sin(zen)
    return np.stack([sz * np.cos(azi), sz * np.sin(azi), cz], -1)


def skyview_dir_to_uv(atm, r, dirs, sun_dir):
    v_horizon = math.sqrt(max(r * r - atm.bottom_radius ** 2, 0.0))
    beta = math.acos(v_horizon / r)
    zenith_horizon = PI - beta
    zen = np.arccos(np.clip(dirs[..., 2], -1, 1))
    upper = zen < zenith_horizon
    # inverse of c_up mapping: coord = 1 - (1-2v)^2  ->  v = 0.5*(1 - sqrt(1 - coord))
    coord = np.clip(zen / zenith_horizon, 0, 1)
    v_up = 0.5 * (1.0 - np.sqrt(1.0 - coord))
    coord_lo = np.clip((zen - zenith_horizon) / max(beta, 1e-6), 0, 1)
    v_lo = 0.5 * (np.sqrt(coord_lo) + 1.0)
    v = np.where(upper, v_up, v_lo)
    # azimuth relative to the sun
    sun_az = math.atan2(sun_dir[1], sun_dir[0])
    az = np.arctan2(dirs[..., 1], dirs[..., 0]) - sun_az
    az = np.abs(np.mod(az + PI, 2 * PI) - PI)  # 0..pi, symmetric about the sun
    u = np.sqrt(az / PI)
    return u, v


def build_skyview(atm, lut_t, lut_ms, sun_elev_deg, cam_alt_km=0.0002, w=192, h=108, steps=30):
    r = atm.bottom_radius + cam_alt_km
    u = from_sub_uv_to_unit((np.arange(w) + 0.5) / w, w)
    v = from_sub_uv_to_unit((np.arange(h) + 0.5) / h, h)
    uu, vv = np.meshgrid(np.clip(u, 0, 1), np.clip(v, 0, 1))
    dirs = skyview_uv_to_dir(atm, r, uu, vv)
    e = math.radians(sun_elev_deg)
    sun = np.array([math.cos(e), 0.0, math.sin(e)])
    L, _, _ = integrate(atm, lut_t, lut_ms, r, dirs, sun, steps, ground=False)
    return L, sun


def sample_skyview(atm, lut_sv, r, dirs, sun_dir):
    u, v = skyview_dir_to_uv(atm, r, dirs, sun_dir)
    u = from_unit_to_sub_uv(u, lut_sv.shape[1])
    v = from_unit_to_sub_uv(v, lut_sv.shape[0])
    return sample_bilinear(lut_sv, u, v)


def sun_transmittance(atm, lut_t, sun_elev_deg, cam_alt_km=0.0002):
    r = atm.bottom_radius + cam_alt_km
    mu = math.sin(math.radians(sun_elev_deg))
    t = transmittance_to_top(atm, lut_t, np.array(r), np.array(mu))
    # Planet occlusion of the sun disk centre.
    t_e, _ = ray_sphere(np.array(r), np.array(mu), atm.bottom_radius)
    if np.isfinite(t_e) and t_e > 0:
        return np.zeros(3)
    return np.asarray(t)
