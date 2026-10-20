# Contributing

Thank you for helping build Jenga Dunia. This project is developed by humans and AI agents under the same rules.

## Before you start
1. Read `memory/session_start.md` and follow its onboarding order.
2. Check `memory/todo.md` for open items and ownership; claim an item by adding your handle.
3. For any architectural change, write or update an ADR in `docs/adr/` first.

## Workflow
- Branch from `main`; small, focused PRs.
- Conventional commits (`feat:`, `fix:`, `docs:`, `perf:`, `refactor:`, `test:`, `chore:`).
- Every PR: CI green on macOS, Linux and Windows; tests for new logic; benchmarks for hot-path changes; docs/matrices updated with the code.
- Review loop before pushing: correctness review and over-engineering review (`/code-review`, `/ponytail-review` for agents). Fix findings first.
- Never bypass hooks or gates (`--no-verify` is prohibited). A broken gate is a finding to report.
- `scripts/pre-push.sh` runs every CI gate locally (fmt, clippy, tests, cargo-deny, schema and TypeScript-type freshness, version, links, frontend build/tests/e2e, and the code knowledge graph when `graphify` is installed). Run it, read the result, then push — never chain `git push` behind a step that can fail silently.
- Fuzzing: `fuzz/` holds cargo-fuzz targets for the parsers we own (`fbx_mesh`, `scatter_csv`, `graph_json`); `cargo +nightly fuzz run <target> -- -max_total_time=60` locally (nightly + `cargo install cargo-fuzz`), CI runs each for 30 s on every push. A crash artifact under `fuzz/artifacts/` is a bug: turn it into a unit test in the parser's module.
- Unity check: `scripts/unity-import-check.sh <profile dir>` rebuilds an exported Unity profile in a scratch Unity project headlessly and prints Unity's report (`integrations/unity/`); the acceptance scenario runs it when a Unity editor is installed.
- Knowledge graph: `scripts/graphify-code.py` builds `graphify-out/` (ignored by git) from the code alone — graphify's AST extraction, no LLM, no key — and `--check` proves the build deterministic by building twice; CI uploads it as the `graphify-out` artifact. Query it with `graphify query "<question>"`, `graphify path "A" "B"`, `graphify explain "X"`.

## Code standards
- Rust: `cargo fmt`, `cargo clippy -D warnings`, `cargo deny check`, `cargo nextest run`. `unsafe` requires a `// SAFETY:` comment and an entry in `docs/architecture/10-security.md`.
- TypeScript: strict mode, ESLint, Prettier, Vitest.
- Deliberate simplifications are marked `// ponytail: <ceiling> — <upgrade path>`.
- Business logic lives in `jd-core`/domain crates only; adapters (Tauri, CLI, MCP) marshal.
- Errors use the catalogue in `docs/spec/error-codes.md`.

## Licensing
- Core is MIT. Do not add GPL/AGPL/proprietary code to core crates. New dependencies need an entry in `docs/research/licensing-register.md` with licence verified from the source.
- Never copy code from reference tools or reverse-engineered artefacts. See `docs/research/opensource-rea.md` for the REA policy.

## Reporting results
State what was run and the real output. "Tests pass" without output is not acceptable. Unavailable tooling (Blender, FBX SDK, engines) → mark `UNVERIFIED`.
