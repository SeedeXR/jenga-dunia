# Jenga Dunia

Open-source (MIT), commercially usable procedural world generation and environmental simulation — Rust core, Tauri desktop app, CLI and MCP server sharing one engine.

**Status: Phase 1 in progress** — Rust workspace with project format, operation registry, `jd` CLI and `jd-mcp` MCP server (28 tests: unit, property, CLI-driven, MCP-driven). Terrain generation, exports and the desktop app follow in Phase 2. See [`docs/user/`](docs/user/README.md).

```sh
cargo build --release -p jd-cli -p jd-mcp
./target/release/jd new ~/Worlds/Hills.jdproj && ./target/release/jd diag env
```

## Start here

| Audience | Read |
|---|---|
| Everyone | [`docs/README.md`](docs/README.md) — index of the single source of truth |
| Agents / contributors | [`memory/session_start.md`](memory/session_start.md) — mandatory onboarding |
| Architecture | [`docs/architecture/00-overview.md`](docs/architecture/00-overview.md) |
| Decisions | [`docs/adr/`](docs/adr/) |
| Research & evidence | [`docs/research/`](docs/research/) |
| Roadmap | [`docs/roadmap.md`](docs/roadmap.md) |

## Principles

One Rust core; GUI, CLI and MCP are thin adapters. Non-destructive node graph with deterministic seeds. Geometry correctness over appearance. Interoperability proven by validators, not menus. Measured performance, ≤ 4 GB RAM. Agent-first automation. Clean-room, licence-verified, commercially safe.

## Stack

Tauri 2 · Rust · TypeScript · Tailwind CSS · Lucide · Host Grotesk · wgpu/WGSL · clap · rmcp (MCP) · versioned JSON schemas. Targets macOS (Apple Silicon first), Windows, Linux.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Licence: [MIT](LICENSE).
