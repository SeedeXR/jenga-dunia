# 2026-10-22 — Unreal headless check: where the 7.5-minute first run went

Question: the first `scripts/unreal-import-check.sh` run took 449 s wall (log span 250 s) — a
shader warm-up, as assumed? Method: timed runs of `UnrealEditor-Cmd 5.8.4` (Apple M4, macOS 27.0.1)
on brand-new Blueprint-only projects, `-ExecutePythonScript` of a 4-line smoke script unless noted,
logs under `/tmp/jd-scratch/ue/`. `-ddc=Cold` is the engine's own empty standalone derived-data
cache (`BaseEngine.ini` `[DerivedDataBackendGraph]`: `Cold=(ZenLocal(Sandbox=Cold, Flush=true))`).

| Run | Project | Cache | Wall | Notes |
|-----|---------|-------|------|-------|
| first ever (2026-10-21 19:41) | JDCheck, `-run=pythonscript` | default | 449 s | 627 `InternalLoadLibrary` lines over 247 s (~0.4 s each); shader log: 2 autogen headers written, no compile jobs; one 20 s gap (Interchange module load) |
| Fresh2 | new project | default | 29 s | 0 module-load lines; 20 s in-log |
| cold_base | new project | `-ddc=Cold` | 21 s | empty DDC, still no shader jobs |
| cold_sm6 | + `TargetedRHIs=SF_METAL_SM6` only | `-ddc=Cold` | 19 s | no change worth the config |
| cold_nosc | + `-NoShaderCompile` | `-ddc=Cold` | 12 s, **exit 1** | `Fatal error: NotNull.cpp line 12` |
| warm_cmdlet | Fresh2, `-run=pythonscript` | default | 11 s | fast, but the commandlet asserts in `ImportAssetTasks` (README) |
| warm_full | Fresh2 | default | 17 s | 11.6 s in-log; ~5 s before logging = UBT `ValidatePlatforms` + dotnet + dyld |
| warm_min | `DisableEnginePluginsByDefault` + Python/Interchange | default | 11 s | 8.1 s in-log |
| real check, old `.uproject` | Bench 256², profile / mesh, two passes each | default | 19 / 18 / 17 / 19 s | `jd-unreal-import.json` |
| real check, trimmed `.uproject` | same | default | 9 / 12 / 12 / 9 s | reports identical to the old ones (diff ignoring timings: empty) |

Findings.

- The first-run cost was **not** shader compilation: no compile jobs were logged, and an empty
  DDC reproduces none of it. It was the first load of ~627 of the engine's ~1200 Mach-O libraries
  from a fresh install (macOS validates and pages them in on first open — inferred from the
  per-library time, not observed in a profiler). It is paid once per machine and engine version
  by whichever project launches first; nothing in a project or on the command line avoids it.
- `DisableEnginePluginsByDefault: true` in the scratch `.uproject` is the per-run win: the
  editor mounts only the three plugins the check needs (plus their dependencies), 17–19 s → 9–12 s,
  identical reports. Applied in `scripts/unreal-import-check.sh`.
- Rejected: `-NoShaderCompile` (aborts the editor with `-ExecutePythonScript`), the
  `pythonscript` commandlet (asserts in the importers), trimming `TargetedRHIs` (2 s, config
  churn). The ~3–5 s before the first log line (UnrealBuildTool `-Mode=ValidatePlatforms` under
  dotnet, run on every launch of a project) has no documented skip flag in the 5.8 binaries
  (`strings` of `libUnrealEditor-TargetPlatform.dylib`: only the AutoSDK reasons) — left alone.
