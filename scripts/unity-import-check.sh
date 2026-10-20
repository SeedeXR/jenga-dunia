#!/usr/bin/env bash
# Import a `terrain.export_profile` Unity directory into a scratch Unity project headlessly and
# print Unity's own report. Usage: scripts/unity-import-check.sh <profile dir> [extent_x,extent_z]
#        scripts/unity-import-check.sh --fbx <file.fbx>      # run our FBX through Unity's importer instead
# Needs a Unity editor (UNITY env or the newest under /Applications/Unity/Hub/Editor). Exit 1 on failure.
set -euo pipefail
MODE=profile
if [ "${1:-}" = "--fbx" ]; then MODE=fbx; FBX="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2; else DIR="$(cd "$1" && pwd)"; EXTENT="${2:-}"; fi
UNITY="${UNITY:-$(ls -d /Applications/Unity/Hub/Editor/*/Unity.app/Contents/MacOS/Unity 2>/dev/null | sort -V | tail -1)}"
[ -x "$UNITY" ] || { echo "no Unity editor found (set UNITY)"; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROJ="${JD_UNITY_PROJECT:-$ROOT/target/scratch/UnityJD}"
LOG="$PROJ.log"
if [ ! -d "$PROJ/Assets" ]; then
  "$UNITY" -batchmode -nographics -quit -logFile "$LOG" -createProject "$PROJ"
fi
rm -rf "$PROJ/Assets/JengaDunia"
mkdir -p "$PROJ/Assets/Editor"
cp "$ROOT/integrations/unity/Editor/JengaDuniaImport.cs" "$PROJ/Assets/Editor/"
set +e
if [ "$MODE" = fbx ]; then
  rm -f "$FBX.jd-unity-import.json"
  "$UNITY" -batchmode -nographics -quit -logFile "$LOG" -projectPath "$PROJ" -executeMethod JengaDunia.Import.Fbx -jdFbx "$FBX"
  CODE=$?; REPORT="$FBX.jd-unity-import.json"
else
  rm -f "$DIR/jd-unity-import.json"
  "$UNITY" -batchmode -nographics -quit -logFile "$LOG" -projectPath "$PROJ" -executeMethod JengaDunia.Import.Run -jdDir "$DIR" ${EXTENT:+-jdExtent "$EXTENT"}
  CODE=$?; REPORT="$DIR/jd-unity-import.json"
fi
set -e
if [ -f "$REPORT" ]; then cat "$REPORT"; else echo "no report written; log tail:"; tail -30 "$LOG"; fi
exit $CODE
