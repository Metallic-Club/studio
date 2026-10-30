"""Blender script: renders a 3/4 preview of glTF/GLB models. Used by previews.py; not part of the fetch path.

    blender -b --factory-startup -noaudio --disable-autoexec --python tools/assets/blender_render.py -- job.json

job.json is a list of {"model": path, "out": png}. Only the glTF importer reads the files (no scripts run).
"""
import json
import math
import sys

import bpy
from mathutils import Vector

SIZE = 256


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    try:
        sc.render.engine = "BLENDER_EEVEE"
    except TypeError:
        sc.render.engine = "BLENDER_WORKBENCH"
    sc.render.resolution_x = sc.render.resolution_y = SIZE
    sc.render.resolution_percentage = 100
    sc.render.film_transparent = True
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_mode = "RGBA"
    try:
        sc.eevee.taa_render_samples = 32
    except AttributeError:
        pass
    sc.view_settings.view_transform = "AgX" if "AgX" in [v.identifier for v in
                                                          type(sc.view_settings).bl_rna.properties["view_transform"].enum_items] else "Filmic"
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    bg = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
    bg.inputs[0].default_value = (0.55, 0.58, 0.64, 1)
    bg.inputs[1].default_value = 0.9
    sc.world = world
    return sc


def bounds(objs):
    lo = Vector((1e18,) * 3)
    hi = Vector((-1e18,) * 3)
    for o in objs:
        for c in o.bound_box:
            w = o.matrix_world @ Vector(c)
            lo = Vector(map(min, lo, w))
            hi = Vector(map(max, hi, w))
    return lo, hi


def render(model: str, out: str) -> None:
    sc = reset()
    bpy.ops.import_scene.gltf(filepath=model)
    bpy.context.view_layer.update()
    meshes = [o for o in sc.objects if o.type == "MESH"]
    if not meshes:
        raise RuntimeError("no meshes")
    lo, hi = bounds(meshes)
    center = (lo + hi) / 2
    radius = max((hi - lo).length / 2, 1e-3)
    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = 50
    cam = bpy.data.objects.new("cam", cam_data)
    sc.collection.objects.link(cam)
    fov = 2 * math.atan(cam_data.sensor_width / (2 * cam_data.lens))
    direction = Vector((0.85, -1.25, 0.75)).normalized()
    cam.location = center + direction * (radius / math.sin(fov / 2) * 1.02)
    cam.rotation_euler = (-direction).to_track_quat("-Z", "Y").to_euler()
    cam_data.clip_start = radius * 0.01
    cam_data.clip_end = radius * 100
    sc.camera = cam
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.data.energy = 3.5
    sun.rotation_euler = (math.radians(50), math.radians(10), math.radians(35))
    sc.collection.objects.link(sun)
    sc.render.filepath = out
    bpy.ops.render.render(write_still=True)


def main():
    job = json.loads(open(sys.argv[sys.argv.index("--") + 1], encoding="utf-8").read())
    for item in job:
        try:
            render(item["model"], item["out"])
            print("rendered", item["out"])
        except Exception as e:  # keep going; previews.py shows a placeholder
            print("FAILED", item["model"], e)


main()
