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
