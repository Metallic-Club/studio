"""Lighting presets as data. Field names follow the creator API in docs/research/2026-10-30-lighting.md
(Roblox names where Roblox has one; Metallic additions are marked in the doc). The reference renderer
reads these, so the values in the doc table are the values that produced the PNGs.

Colours are 0-255 sRGB like Roblox Color3.fromRGB. Distances are studs.
"""

DEFAULTS = {
    # Lighting
    "ClockTime": 14.5,
    "GeographicLatitude": 41.73,
    "Brightness": 1.0,
    "ExposureCompensation": 0.0,
    "Ambient": (0, 0, 0),
    "OutdoorAmbient": (0, 0, 0),
    "ColorShift_Top": (0, 0, 0),
    "ColorShift_Bottom": (0, 0, 0),
    "EnvironmentDiffuseScale": 1.0,
    "EnvironmentSpecularScale": 1.0,
    "GlobalShadows": True,
    "ShadowSoftness": 0.2,
    "LightingStyle": "Realistic",
    # Metallic additions on Lighting
    "AutoExposure": True,
    "ExposureMin": -4.0,  # EV100 clamp
    "ExposureMax": 16.0,
    "NightAdaptation": 1.0,  # 0 = no scotopic tint / night darkening bias
    "MoonBrightness": 1.0,
    # Atmosphere (Roblox names)
    "Density": 0.3,
    "Offset": 0.25,
    "Color": (199, 199, 199),
    "Decay": (106, 112, 125),
    "Glare": 2.0,
    "Haze": 1.0,
    # Metallic additions on Atmosphere
    "HeightFalloff": 0.02,  # 1/studs; density halves every ln2/falloff = 35 studs
    "FogHeight": 0.0,  # studs; altitude where Density applies
    "Absorption": (0, 0, 0),  # extra per-channel extinction tint for water/smoke; 0 = none
    "AirColor": (116, 172, 255),  # Rayleigh tint; this default is Earth (blue sky); orange gives a dusty planet
    # Clouds (Roblox has a Clouds object with Cover, Density, Color; only Cover drives lighting here)
    "CloudCover": 0.0,
    # Sky
    "SunAngularSize": 1.5,  # degrees (Roblox default 21 is a texture with glow baked in)
    "MoonAngularSize": 1.5,
    "StarCount": 3000,
    # Post (BloomEffect / ColorCorrectionEffect / ColorGradingEffect)
    "TonemapperPreset": "AgX Punchy",  # API name: Default
    "BloomIntensity": 0.04,
    "BloomSize": 24,  # Roblox BloomEffect.Size in pixels at 1080p
    "BloomThreshold": 0.0,  # 0 = energy-conserving bloom (no threshold)
    "Contrast": 0.0,
    "Saturation": 0.0,
    "TintColor": (255, 255, 255),
    "Vignette": 0.15,
    # Look (render-styles doc): not lighting, but presets pick one
    "Look": "Realistic",
}


def preset(**overrides):
    p = dict(DEFAULTS)
    for k, v in overrides.items():
        if k not in p:
            raise KeyError(k)
        p[k] = v
    return p


PRESETS = {
    "Clear Day": preset(),
    "Golden Hour": preset(
        ClockTime=17.6, Brightness=1.2, ShadowSoftness=0.3, Density=0.35, Haze=2.0, Glare=4.0,
        Color=(230, 196, 160), Decay=(120, 104, 128), TonemapperPreset="AgX Punchy", BloomIntensity=0.06,
        Saturation=0.05, ExposureCompensation=0.3,
    ),
    "Overcast": preset(
        ClockTime=13.0, Brightness=1.0, CloudCover=0.95, ShadowSoftness=1.0, Density=0.45, Haze=4.0, Glare=0.0,
        Color=(210, 212, 216), Decay=(150, 156, 166), HeightFalloff=0.008, TonemapperPreset="AgX",
        BloomIntensity=0.02, Saturation=-0.05, ExposureCompensation=0.5,
    ),
    "Night City": preset(
        ClockTime=22.5, Brightness=1.0, MoonBrightness=0.6, Density=0.4, Haze=3.0, Glare=1.0,
        Color=(120, 110, 140), Decay=(40, 40, 60), Ambient=(60, 45, 80), OutdoorAmbient=(10, 10, 18),
        TonemapperPreset="AgX Punchy", BloomIntensity=0.08, ExposureCompensation=0.0, StarCount=500,
        NightAdaptation=0.5, HeightFalloff=0.006,
    ),
    "Moonlit": preset(
        ClockTime=1.0, MoonBrightness=1.5, Density=0.2, Haze=0.5, Glare=1.0, ShadowSoftness=0.15,
        TonemapperPreset="AgX", BloomIntensity=0.05, ExposureCompensation=0.5, StarCount=5000,
        NightAdaptation=1.0,
    ),
    "Horror Fog": preset(
        ClockTime=18.15, Brightness=0.4, ShadowSoftness=0.8, Density=0.9, Offset=0.0, Haze=6.0, Glare=0.0,
        Color=(150, 160, 150), Decay=(70, 78, 72), HeightFalloff=0.05, FogHeight=0.0,
        TonemapperPreset="AgX", Saturation=-0.45, Contrast=0.15, TintColor=(225, 240, 228), Vignette=0.45,
        ExposureCompensation=-0.6, BloomIntensity=0.03, StarCount=0,
    ),
    "Sci-fi Interior": preset(
        ClockTime=14.5, Brightness=1.0, Density=0.05, Haze=0.0, Ambient=(30, 34, 46), OutdoorAmbient=(0, 0, 0),
        EnvironmentDiffuseScale=0.3, EnvironmentSpecularScale=1.0, ShadowSoftness=0.1,
        TonemapperPreset="AgX Punchy", BloomIntensity=0.07, Contrast=0.05, TintColor=(235, 245, 255),
        ExposureCompensation=0.3,
    ),
    "Cartoon Bright": preset(
        ClockTime=13.0, Brightness=1.3, ShadowSoftness=0.05, Density=0.15, Offset=0.5, Haze=0.0, Glare=1.0,
        OutdoorAmbient=(40, 44, 60), ColorShift_Top=(20, 12, 0), EnvironmentDiffuseScale=0.7,
        TonemapperPreset="PBR Neutral", Saturation=0.15, BloomIntensity=0.02, Vignette=0.0,
        LightingStyle="Soft", Look="Toon", ExposureCompensation=0.2,
    ),
    "Pixel Dusk": preset(
        ClockTime=17.8, Brightness=1.0, ShadowSoftness=0.0, Density=0.3, Haze=2.0, Glare=3.0,
        Color=(255, 170, 120), Decay=(90, 70, 130), TonemapperPreset="PBR Neutral", BloomIntensity=0.0,
        Saturation=0.1, Vignette=0.0, Look="Pixel 3D", ExposureCompensation=0.0, SunAngularSize=4.0,
    ),
    "Underwater": preset(
        ClockTime=13.0, Brightness=0.6, ShadowSoftness=0.6, Density=1.0, Offset=0.0, Haze=0.0, Glare=0.0,
        Color=(40, 170, 180), Decay=(10, 50, 90), HeightFalloff=0.0, Absorption=(230, 90, 40),
        TonemapperPreset="AgX", Saturation=0.05, BloomIntensity=0.06, Vignette=0.35, ExposureCompensation=0.3,
        StarCount=0,
    ),
    "Desert Noon": preset(
        ClockTime=12.3, GeographicLatitude=25.0, Brightness=1.6, ShadowSoftness=0.1, Density=0.3, Haze=3.0,
        Glare=3.0, Color=(240, 210, 170), Decay=(170, 150, 130), TonemapperPreset="AgX Punchy", Contrast=0.05,
        Saturation=0.05, TintColor=(255, 246, 232), ExposureCompensation=-0.2,
    ),
    "Alien Dusk": preset(
        ClockTime=17.85, Brightness=1.0, Density=0.4, Haze=6.0, Glare=6.0, Color=(120, 170, 255),
        Decay=(200, 120, 70), AirColor=(255, 170, 110), TonemapperPreset="AgX Punchy", BloomIntensity=0.06,
    ),
}

# Per-preset scene lights for the reference renderer (PointLight: position, Color, Brightness, Range)
# and Neon parts. These are the scene, not the preset; listed here so the renders are reproducible.
SCENE_LIGHTS = {
    "Night City": [((-12, 9, 6), (255, 190, 120), 0.6, 30), ((14, 7, 2), (120, 180, 255), 0.6, 28),
                   ((0, 4, -8), (255, 120, 200), 0.4, 22)],
    "Sci-fi Interior": [((0, 11, 10), (190, 220, 255), 1.5, 40), ((-14, 5, 14), (120, 255, 230), 0.8, 24),
                        ((14, 5, 14), (255, 140, 90), 0.8, 24)],
    "Horror Fog": [((10, 4, 0), (255, 150, 80), 1.2, 22)],
}
