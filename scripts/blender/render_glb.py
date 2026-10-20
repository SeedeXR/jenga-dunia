# Render a Jenga Dunia GLB from an oblique camera with a low sun — the documentation renders.
# Usage: /Applications/Blender.app/Contents/MacOS/Blender -b --python scripts/blender/render_glb.py -- in.glb out.png [azimuth_deg] [elevation_deg] [sky.exr]
# Workbench + EEVEE are not needed: Cycles at 64 samples on CPU renders a 1 M-triangle terrain in well under a minute.
import bpy, sys, math
args = sys.argv[sys.argv.index("--") + 1:]
glb, out = args[0], args[1]
az = math.radians(float(args[2]) if len(args) > 2 else 35.0)
el = math.radians(float(args[3]) if len(args) > 3 else 28.0)
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
objs = [o for o in bpy.data.objects if o.type == "MESH"]
import mathutils
corners = [o.matrix_world @ mathutils.Vector(c) for o in objs for c in o.bound_box]
lo = mathutils.Vector((min(c.x for c in corners), min(c.y for c in corners), min(c.z for c in corners)))
hi = mathutils.Vector((max(c.x for c in corners), max(c.y for c in corners), max(c.z for c in corners)))
centre = (lo + hi) / 2
size = max(hi.x - lo.x, hi.y - lo.y)
scene = bpy.context.scene
cam_data = bpy.data.cameras.new("cam"); cam_data.lens = 35; cam_data.clip_end = size * 10
cam = bpy.data.objects.new("cam", cam_data); scene.collection.objects.link(cam)
dist = size * 1.15
cam.location = centre + mathutils.Vector((dist * math.cos(el) * math.sin(az), -dist * math.cos(el) * math.cos(az), dist * math.sin(el) + (hi.z - lo.z) * 0.3))
cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
sun_data = bpy.data.lights.new("sun", "SUN"); sun_data.energy = 4.0; sun_data.angle = math.radians(1.5)
sun = bpy.data.objects.new("sun", sun_data); scene.collection.objects.link(sun)
sun.rotation_euler = (math.radians(62), 0.0, math.radians(140))
world = bpy.data.worlds.new("w"); scene.world = world; world.use_nodes = True
bg = world.node_tree.nodes["Background"]; bg.inputs[0].default_value = (0.55, 0.65, 0.8, 1.0); bg.inputs[1].default_value = 0.6
hdri = args[4] if len(args) > 4 else None  # optional equirectangular EXR: lights the scene instead of the sun lamp
if hdri:
    env = world.node_tree.nodes.new("ShaderNodeTexEnvironment"); env.image = bpy.data.images.load(hdri)
    world.node_tree.links.new(env.outputs["Color"], bg.inputs["Color"]); bg.inputs[1].default_value = 1.0
    sun_data.energy = 0.0
scene.render.engine = "CYCLES"; scene.cycles.samples = 128 if hdri else 64; scene.cycles.device = "CPU"
scene.render.resolution_x = 1200; scene.render.resolution_y = 700; scene.render.resolution_percentage = 100
scene.render.filepath = out; scene.render.image_settings.file_format = "PNG"
bpy.ops.render.render(write_still=True)
print("RENDERED " + out + " triangles=" + str(sum(len(o.data.polygons) for o in objs)))
