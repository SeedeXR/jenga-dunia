#!/usr/bin/env bash
# Import a Jenga Dunia export into a scratch Unreal project headlessly and print Unreal's own
# report. Usage: scripts/unreal-import-check.sh <unreal profile dir>
#        scripts/unreal-import-check.sh --mesh <terrain.glb|terrain.fbx>
# Needs an Unreal Engine 5 install (UNREAL env = the UnrealEditor-Cmd binary, else the newest under
# /Users/Shared/Epic Games). The scratch project is a Blueprint-only .uproject written here; the
# first run warms the engine's shader cache (minutes), later runs take ~1 min. The script runs through
# -ExecutePythonScript (full editor, unattended): the pythonscript commandlet asserts inside the
# asset importers. Exit 1 on failure.
set -euo pipefail
MODE=profile
if [ "${1:-}" = "--mesh" ]; then MODE=mesh; MESH="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2; else DIR="$(cd "$1" && pwd)"; fi
UNREAL="${UNREAL:-$(ls -d "/Users/Shared/Epic Games"/UE_*/Engine/Binaries/Mac/UnrealEditor-Cmd 2>/dev/null | sort -V | tail -1)}"
[ -x "$UNREAL" ] || { echo "no Unreal editor found (set UNREAL)"; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="${JD_UNREAL_PROJECT:-$ROOT/target/scratch/UnrealJD}"
VER="$(python3 -c "import json;print(json.load(open('$(dirname "$(dirname "$(dirname "$(dirname "$UNREAL")")")")/Build/Build.version'))['MajorVersion'], end='.')" 2>/dev/null || echo 5.)$(python3 -c "import json;print(json.load(open('$(dirname "$(dirname "$(dirname "$(dirname "$UNREAL")")")")/Build/Build.version'))['MinorVersion'])" 2>/dev/null || echo 8)"
mkdir -p "$PROJ/Content" "$PROJ/Saved"
cat > "$PROJ/UnrealJD.uproject" <<JSON
{ "FileVersion": 3, "EngineAssociation": "$VER", "Category": "", "Description": "Jenga Dunia import check",
  "Plugins": [ { "Name": "PythonScriptPlugin", "Enabled": true }, { "Name": "Interchange", "Enabled": true }, { "Name": "InterchangeEditor", "Enabled": true } ] }
JSON
LOG="$PROJ.log"
set +e
if [ "$MODE" = mesh ]; then
  REPORT="$MESH.jd-unreal-import.json"; rm -f "$REPORT"
  JD_MESH="$MESH" JD_OUT="$REPORT" "$UNREAL" "$PROJ/UnrealJD.uproject" -ExecutePythonScript="$ROOT/integrations/unreal/jd_import_check.py" -unattended -nop4 -nosplash -NullRHI -stdout > "$LOG" 2>&1
else
  REPORT="$DIR/jd-unreal-import.json"; rm -f "$REPORT"
  JD_PROFILE="$DIR" JD_OUT="$REPORT" "$UNREAL" "$PROJ/UnrealJD.uproject" -ExecutePythonScript="$ROOT/integrations/unreal/jd_import_check.py" -unattended -nop4 -nosplash -NullRHI -stdout > "$LOG" 2>&1
fi
CODE=$?
set -e
if [ -f "$REPORT" ]; then cat "$REPORT"; python3 -c "import json,sys; sys.exit(0 if json.load(open('$REPORT')).get('ok') else 1)"; else echo "no report written; log tail:"; grep -E 'LogPython|Error' "$LOG" | tail -20; exit 1; fi
