# ADR-0004 — Frontend framework and node-editor library

Status: Proposed (confirm at Phase 2 start) · 2026-10-19

## Context
Brief §2 fixes TypeScript + Tailwind + Lucide + Host Grotesk but leaves the framework open. The deciding component is the node editor (`stack.md` §3): React Flow 12.12 (MIT) is the most maintained and documented with custom nodes/edges and virtualisation; Svelte Flow is the same team with a smaller ecosystem; Rete.js is slower-moving; baklavajs is Vue-only; litegraph.js is dead upstream.

## Decision
React 19 + React Flow (`@xyflow/react` 12) + Zustand + Tailwind 4 + shadcn/ui (Radix) + `lucide-react` (ISC) + `@fontsource-variable/host-grotesk` (OFL-1.1, self-hosted) + three.js; Vite, TypeScript, Biome, Vitest, Playwright. TypeScript API types are generated from the committed JSON schemas, never hand-written.

## Alternatives rejected
- Svelte 5 + Svelte Flow — viable; smaller ecosystem for the many panels needed.
- Vue + baklavajs — editor less capable.
- Custom canvas node editor — unjustified reinvention.

## Consequences
- React Flow performance with hundreds of nodes must be verified (virtualisation on).
- shadcn/ui Tailwind v4 support is REPORTED; verify at setup.

## Evidence
`docs/research/stack.md` §3 (versions verified on npm 2026-10-19).
