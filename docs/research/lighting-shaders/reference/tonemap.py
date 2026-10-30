"""CPU twins of ../tonemap.wgsl. Input: linear Rec.709 scene light after exposure. Output: linear display
Rec.709 in [0, 1] (encode with srgb_encode for an 8-bit PNG)."""
from __future__ import annotations

import numpy as np

# ---- AgX (Troy Sobotka), minimal polynomial form (Benjamin Wrensch) -------------------------
# Matrices as GLSL mat3 column lists; M below is the row-major matrix so that out = M @ c.
_AGX_COLS = [
    [0.842479062253094, 0.0423282422610123, 0.0423756549057051],
    [0.0784335999999992, 0.878468636469772, 0.0784336],
    [0.0792237451477643, 0.0791661274605434, 0.879142973793104],
]
_AGX_INV_COLS = [
    [1.19687900512017, -0.0528968517574562, -0.0529716355144438],
    [-0.0980208811401368, 1.15190312990417, -0.0980434501171241],
    [-0.0990297440797205, -0.0989611768448433, 1.15107367264116],
]
AGX_MAT = np.array(_AGX_COLS).T
AGX_INV = np.array(_AGX_INV_COLS).T
AGX_MIN_EV = -12.47393
AGX_MAX_EV = 4.026069


def _agx_curve(x):
    x2 = x * x
    x4 = x2 * x2
    return (15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - 0.00232)


def agx(c, look="none"):
    c = np.maximum(c, 1e-10) @ AGX_MAT.T
    c = np.clip(np.log2(np.maximum(c, 1e-10)), AGX_MIN_EV, AGX_MAX_EV)
    c = (c - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV)
    c = _agx_curve(c)
    if look != "none":
        if look == "punchy":
            slope, offset, power, sat = np.array([1.0] * 3), 0.0, np.array([1.35] * 3), 1.4
        elif look == "golden":
            slope, offset, power, sat = np.array([1.0, 0.9, 0.5]), 0.0, np.array([0.8] * 3), 0.8
        else:
            raise ValueError(look)
        c = np.power(np.maximum(c * slope + offset, 0.0), power)
        luma = (c * np.array([0.2126, 0.7152, 0.0722])).sum(-1, keepdims=True)
        c = luma + sat * (c - luma)
    c = c @ AGX_INV.T
    # AgX output is display-referred with a 2.2 power; return linear display light.
    return np.power(np.clip(c, 0.0, 1.0), 2.2)


# ---- Khronos PBR Neutral ---------------------------------------------------------------------

def pbr_neutral(c):
    c = np.array(c, dtype=np.float64)
    start = 0.8 - 0.04
    desat = 0.15
    x = c.min(-1, keepdims=True)
    offset = np.where(x < 0.08, x - 6.25 * x * x, 0.04)
    c = c - offset
    peak = c.max(-1, keepdims=True)
    d = 1.0 - start
    new_peak = 1.0 - d * d / (peak + d - start)
    scaled = c * (new_peak / np.maximum(peak, 1e-9))
    g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0)
    comp = scaled * (1 - g) + new_peak * g
    return np.clip(np.where(peak < start, c, comp), 0.0, 1.0)


# ---- ACES fits ---------------------------------------------------------------------------------

def aces_narkowicz(c):
    x = np.maximum(c, 0.0)
    return np.clip((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0)


_ACES_IN = np.array([[0.59719, 0.35458, 0.04823], [0.07600, 0.90834, 0.01566], [0.02840, 0.13383, 0.83777]])
_ACES_OUT = np.array([[1.60475, -0.53108, -0.07367], [-0.10208, 1.10813, -0.00605],
                      [-0.00327, -0.07276, 1.07602]])


def aces_hill(c):
    v = np.maximum(c, 0.0) @ _ACES_IN.T
    a = v * (v + 0.0245786) - 0.000090537
    b = v * (0.983729 * v + 0.4329510) + 0.238081
    return np.clip((a / b) @ _ACES_OUT.T, 0.0, 1.0)


def reinhard_luma(c):
    l = (np.maximum(c, 0) * np.array([0.2126, 0.7152, 0.0722])).sum(-1, keepdims=True)
    return np.clip(c / (1.0 + l), 0, 1)


def clamp_curve(c):
    return np.clip(c, 0.0, 1.0)


TONEMAPPERS = {
    "AgX": agx,
    "AgX Punchy": lambda c: agx(c, "punchy"),
    "PBR Neutral": pbr_neutral,
    "ACES (Hill)": aces_hill,
    "ACES (Narkowicz)": aces_narkowicz,
    "Clamp": clamp_curve,
}


def srgb_encode(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)


def srgb_decode(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, np.power((c + 0.055) / 1.055, 2.4))


def to_png_bytes(img_linear):
    return (srgb_encode(img_linear) * 255.0 + 0.5).astype(np.uint8)
