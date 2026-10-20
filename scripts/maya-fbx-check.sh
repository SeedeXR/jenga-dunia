#!/usr/bin/env bash
# Import an FBX through Maya's FBX plugin headlessly (the Autodesk FBX SDK) and print Maya's report.
# Usage: scripts/maya-fbx-check.sh <file.fbx>   (MAYAPY env or the newest /Applications/Autodesk/maya*/Maya.app/Contents/bin/mayapy)
set -euo pipefail
FBX="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
MAYAPY="${MAYAPY:-$(ls -d /Applications/Autodesk/maya*/Maya.app/Contents/bin/mayapy 2>/dev/null | sort -V | tail -1)}"
[ -x "$MAYAPY" ] || { echo "no mayapy found (set MAYAPY)"; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
"$MAYAPY" "$ROOT/integrations/maya/fbx_check.py" "$FBX" 2>&1 | grep -E "^JD_MAYA_FBX" | sed 's/^JD_MAYA_FBX //'
test -f "$FBX.jd-maya-import.json" && grep -q '"ok": true' "$FBX.jd-maya-import.json"
