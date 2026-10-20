# Maya FBX check (headless)

`fbx_check.py` imports an FBX through Maya's FBX plugin — the Autodesk FBX SDK — in
`mayapy` (no UI, no licence prompt on this machine) and writes `<file>.jd-maya-import.json`:
meshes with vertex / face / triangle counts, UV sets and world bounds in Maya's linear unit
(centimetres by default, so a metre-authored file reads ×100), plus units and up axis.

```
scripts/maya-fbx-check.sh exports/terrain.fbx
```

The acceptance scenario (`scripts/acceptance.mjs`) runs it when a `mayapy` is found under
`/Applications/Autodesk/maya*` and marks item 11 from the report. Verified with Maya 2027 /
FBX plugin 2020.3.9. MIT, like the rest of the repository.
