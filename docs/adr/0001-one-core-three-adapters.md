# ADR-0001 — One Rust core, thin GUI/CLI/MCP adapters

Status: Accepted · 2026-10-19

## Context
The brief (§3.2, §13) requires GUI, CLI and MCP to expose the same functionality with no duplicated logic, and automation from Phase 1. World Creator has no automation at all; Gaea and World Machine gate CLI/automation behind Pro tiers and bolt them onto GUI-centric cores (`docs/research/gaea-world-machine.md` D4). TerraForge3D's JSON-schema'd HTTP API + `SKILL.md` shows the agent-facing pattern working in an open-source tool (`opensource-rea.md` §1).

## Decision
`jd-core` holds an **operation registry** (`op id → request/result schema, handler, scope, async`). Tauri commands, CLI subcommands and MCP tools are generated or thinly written over it. Adapters contain no domain logic. Schemas are generated with `schemars`, committed, and used to generate TypeScript types.

## Alternatives rejected
- Separate feature implementations per interface — guaranteed drift; explicitly prohibited by the brief.
- GUI-first with CLI later — reproduces the incumbents' weakness; agent support is a differentiator.
- A single long-running daemon that all adapters talk to — adds IPC complexity before it is needed; revisit in Phase 5 (`01-system-architecture.md` process model).

## Consequences
- Every feature's definition of done includes CLI and MCP exposure (`memory/instruction.md` §9).
- Frontend state is a mirror of the document; mutations round-trip through ops.
- Slight overhead per op (serialisation) — acceptable; large payloads go through channels, not JSON.

## Evidence
`docs/research/world-creator.md` §6; `docs/research/gaea-world-machine.md` §D4 items 25–29; `docs/research/opensource-rea.md` §1 (TF3D API).
