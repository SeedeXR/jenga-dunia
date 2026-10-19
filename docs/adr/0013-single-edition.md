# ADR-0013 — One edition, no feature tiers

**Status:** Accepted · 2026-10-20

## Context

Gaea and World Machine sell Community/Indie/Professional editions that gate automation, tiled builds, output resolution, thread counts and commercial use behind price tiers (`docs/research/gaea-world-machine.md`). The brief positions Jenga Dunia as an open-source, commercially usable alternative in the spirit of Blender, and the owner has decided explicitly: no community or professional edition.

## Decision

Jenga Dunia ships **one edition**. Every build contains every feature: GUI, CLI, MCP automation, tiled builds, full resolution, all node kinds, all exporters. There are no licence keys, activation, seat counts, resolution caps or "Pro only" flags anywhere in the core, the adapters or the documentation. Optional adapters exist only for *legal* reasons (copyleft or proprietary SDKs kept outside the MIT core, ADR-0010/`philosophy.md` §8), never for commercial segmentation.

## Consequences

- No code path may branch on an edition, licence or entitlement; reviewers reject such flags.
- Docs and marketing copy never describe tiers; comparisons with competitors' editions stay in `docs/research/`.
- Funding, if any, follows the Blender model (donations, development fund, services), which is outside this repository.
