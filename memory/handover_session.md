# Handover Sessions

Append-only log. Newest entry first. Timestamps are local (Africa/Nairobi, UTC+3) in `YYYY-MM-DD HH:MM:SS`.

---

## Session 2 — 2026-10-19 — Phase 1 code: workspace, project format, op registry, CLI, MCP

**Mode:** `/loop` dynamic (owner: "don't stop until everything is done"). **Owner additions this session:** platform-native mandate (Metal on macOS, DX12/Vulkan + NVIDIA on Windows/Linux, CPU everywhere; measure on this Mac); Figma file `fxyeV62hgLoNmG3dU1LX6G` for annotated tutorial screenshots; UI to follow World Creator's workflow.

### Timeline
- `17:05:00` Verified crate versions on crates.io (`cargo info`); read rmcp 3.5.1 and ulid sources from the registry instead of guessing APIs (ulid 3 removed `Ulid::new` → pinned ulid 1; rmcp `Tool`/`Implementation`/`CallToolRequestParams` are non-exhaustive → constructors).
- `17:20:00` Wrote `crates/jd-types` (ErrorCode catalogue + exit codes, JdError envelope, ULID ids, SchemaVersion, Units/WorldSettings with proptest), `crates/jd-core` (project directory v1 with atomic writes, unknown-key preservation, schema gate, WAL journal + recovery, pid lock; op registry with schemars schemas; envelope; diag.env), `apps/cli` (`jd new|open|op|schema|diag env`, `--json`, exit codes), `apps/mcp` (manual `ServerHandler`: tools from registry, scope gate, workspace confinement, structured results).
- `17:50:00` Tests: 7 (types) + 13 (core incl. insta snapshot + jsonschema validation) + 5 CLI-driven (assert_cmd on the real binary) + 3 MCP-driven (rmcp client ↔ spawned `jd-mcp` over stdio) = **28 passing** (`cargo test --workspace`). `cargo-nextest` failed to install → using `cargo test`.
- `18:05:00` Gates: `cargo fmt --check` ok; `cargo clippy --workspace --all-targets -D warnings` ok (clippy.toml allows expect/unwrap in tests; `result_large_err` allowed by design); `cargo deny check` ok (deny.toml: licence allow-list, bincode≥3/contour-isobands/openssl-sys banned, crates `publish = false`).
- `18:10:00` Ponytail review applied (−16 lines: `as_str` round-trip, dead fn, `is_async`). Correctness review (`/code-review medium`) was interrupted by an API session limit on the previous account, resumed after re-login.
- CI workflow now has `docs`, `rust` (3 OS: fmt/clippy/test/release build/CLI smoke) and `cargo-deny` jobs; actions bumped to checkout@v5 / setup-node@v5; npx cache for mermaid-cli.

### Decisions
- Adapters are thin: CLI subcommands and MCP tools call `Registry::invoke` with the JSON envelope (ADR-0001 realised). Generic `jd op <id> --params` guarantees every op is scriptable.
- Workspace confinement and scope gating live in `jd-mcp` (adapter policy) — `ponytail:` move into `jd-core` when the GUI needs the same gate.
- `project.recover` replays through the same `apply_mutation` used by live ops so recovery cannot diverge.

### Open / next
1. Wait for code-review findings; fix; commit + push (CI will run Rust matrix for the first time — watch Windows path handling in the CLI smoke step).
2. Phase 2 start: `jd-terrain` tiles + noise nodes, `jd-graph` evaluator, `jd-jobs`; first `graph.evaluate` + EXR/PNG16/R16 export; viewport prototype (ADR-0003 gate).
3. `docs/user/`: CLI quickstart written from real output; GUI tutorials with Figma-annotated screenshots/video once the Tauri app exists.

---

## Session 1 — 2026-10-19 — Round 1: research + single source of truth

**Agent:** Claude (Fable 5.1 / Opus 5.5) orchestrating 8 research sub-agents. **Human:** project owner.

### Objective
Produce the documentation single source of truth before any code: research with evidence, architecture with Mermaid, ADRs, specs, memory files, governance, CI for docs.

### Timeline
- `2026-10-19 11:05:00` Brief read (`docs/research/brief/WorldCreator-JengaDunia.md`, 1,577 lines). Repo cloned: only `LICENSE` (MIT) present. Toolchain observed: rustc 1.98.1, cargo 1.98.1, node 26.11.1, pnpm 10.33.2, python 3.14.6, Apple USD Tools 0.25.11 (`usdcat`); no cargo-tauri, blender, gltf-validator on PATH.
- `2026-10-19 11:10:00` Launched 7 research agents (World Creator; Gaea + World Machine; open-source tools + REA; formats/SDKs; Rust/Tauri/wgpu stack; algorithms; geo + vision). Reports written to scratchpad and copied to `docs/research/`.
- `2026-10-19 11:15:00`–`13:30:00` Wrote `memory/` files, architecture docs 00–13, ADRs 0001–0012, specs, research syntheses (competitive matrix, licensing register, evidence register), roadmap, README/CONTRIBUTING/SECURITY/CoC, validation scripts, CI workflow.
- `2026-10-19 13:05:00` User added scope: geographically accurate soil/sand/rock texture references, water/sky/wind/snow shader techniques, soil/dust particle physics, geomorphological process references. Launched 8th agent → `docs/research/materials-geomorphology.md` (pending at time of writing; see todo).
- `2026-10-19 13:35:00` Spot-check: World Creator level-step claim verified against official docs page. Mermaid validation: 29/29 blocks render (fixed: reserved id `graph`, semicolons in sequence messages). Link check: all resolve except the pending materials report.

### Decisions made (recorded as ADRs)
0001 one core/three adapters · 0002 DAG + stack view · 0003 viewport = three.js WebGPU/WebGL2 in WebView, Rust compute headless (Proposed, prototype gate) · 0004 React + React Flow (Proposed) · 0005 f32 tiles with halos · 0006 erosion order (thermal → pipes → droplet → stream power) · 0007 determinism contract · 0008 performance methodology, 4 GB gate · 0009 out-of-process plugins · 0010 REA/clean-room policy · 0011 project as directory · 0012 format strategy (Proposed).

Notable corrections during the session: merged planned `jd-render`/`jd-compute` into a single `jd-gpu` crate (wgpu ships breaking majors quarterly; viewport moved to WebView per research).

### Verification run
- `node scripts/check-mermaid.mjs` → `29/29 mermaid blocks valid`
- `node scripts/check-links.mjs` → `56/57 relative links resolve` (missing: `docs/research/materials-geomorphology.md`, agent still running)
- No code → no cargo/test gates applicable. CI workflow (`.github/workflows/ci.yml`) runs the two scripts on macOS/Ubuntu/Windows; first run will happen on push.

### Open items / next session
1. When the materials-geomorphology report lands: copy to `docs/research/`, update evidence register row, add material presets to `04-graph-engine.md` node catalogue if warranted, commit.
2. Push to `SeedeXR/jenga-dunia` main (brief authorises); confirm CI green on the three runners (first-run Chromium download for mermaid-cli may be slow — consider caching `~/.npm/_npx`).
3. Phase 1 backlog in `memory/todo.md`: workspace skeleton, project format v1, op registry + schemas, CLI/MCP foundations, viewport prototype gate (ADR-0003), CI Rust matrix.
4. Research follow-ups in `docs/research/evidence-register.md` (MoGe-2 weight licence file, WebView2 WebGPU default, `openusd` usdz alignment, Unity RAW endianness, Mei 2007 PDF).
5. Generate the `graphify` knowledge graph once the repo has code (deterministic; add to session-end checklist).

### Addendum `2026-10-19 16:30:00` — materials report, CI, installer inventory
- Materials/geomorphology report integrated (`43b7245`); reference-layer architecture added to 02/04.
- CI run 1 failed only on the link check (materials report missing at that commit); run 2 on `43b7245` **green on macOS, Ubuntu, Windows** (Mermaid render + link check). Warning: actions v4 pinned to Node 20 → todo.
- Owner added `installables/` (git-ignored) with World Machine Basic 4067 `.pkg` and Gaea 2.3.1.0 `.exe` and asked for REA analysis. Performed **inventory-only** inspection with native tools per ADR-0010 (no decompilation; REA not installed/run — not present, VM required, scope covered). Log: `docs/research/research-log/2026-10-19-installer-inventory.md`; addendum in `gaea-world-machine.md`; evidence register row. Gaea payload needs Inno Setup 7 support (innoextract HEAD build attempted).

### Files touched
`README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `.gitignore`, `.github/workflows/ci.yml`, `scripts/check-mermaid.mjs`, `scripts/check-links.mjs`, `memory/*` (9 files), `docs/README.md`, `docs/roadmap.md`, `docs/architecture/00–13`, `docs/adr/README.md + 0001–0012`, `docs/spec/*` (5), `docs/research/*` (10 + brief).
