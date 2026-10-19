# ADR-0010 — REA usage and clean-room policy

Status: Accepted · 2026-10-19

## Context
The brief (§1.3) mandates using REA (https://github.com/morluto/rea) as an authorised research instrument while respecting licences, EULAs and TPMs. Research (`opensource-rea.md` §3) finds REA is an MIT TypeScript MCP server/CLI providing agents with static/dynamic analysis tooling (Ghidra/Rizin/Hopper/IDA bridges, Electron/JS AST, .NET, APK, Frida, CDP, pcap). World Creator's EULA §8.3 prohibits reverse engineering; Gaea/World Machine are proprietary.

## Decision
Adopt the policy in `docs/research/opensource-rea.md` §7 verbatim:
- Permitted: analysis of our own binaries/exports, permissively licensed OSS (read, not copy from copyleft), black-box interoperability observation of engines/DCCs, security research on our releases, provenance checks of contributions.
- Prohibited: decompiling/tracing any proprietary terrain tool or engine to learn algorithms, UI, undocumented formats or assets; bypassing any TPM/licence check; copying or paraphrasing decompiled output; analysing software without authorisation.
- Clean-room rule: if an interoperability exception is ever exercised, the REA operator writes a functional spec only; a different contributor implements; maintainer review; log in `docs/research/research-log/`.
- REA is dev-time only, never a build/runtime dependency; run in an isolated VM.

## Alternatives rejected
- Using REA on competitor binaries "for research" — contractually prohibited (WC EULA) and legally risky; also unnecessary: public docs + papers suffice (this round's reports demonstrate it).

## Consequences
- Competitive knowledge comes from documentation, press and papers; evidence register records provenance.
- Contributions without clean provenance are rejected (`CONTRIBUTING.md`).

## Evidence
`docs/research/opensource-rea.md` §3, §7; `docs/research/world-creator.md` §1.3.
