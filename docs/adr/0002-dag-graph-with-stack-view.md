# ADR-0002 — DAG node graph as source of truth; stack view as projection

Status: Accepted · 2026-10-19

## Context
World Creator uses a hierarchical layer/filter stack praised for approachability but is adding a node view in WC4 after a decade (`world-creator.md` §2.2, §13). Gaea and World Machine are strict one-directional DAGs with loop constructs, portals, macros, typed ports (`gaea-world-machine.md` D1). The brief §6 mandates a non-destructive node graph independent of the editor.

## Decision
The document model is a typed **DAG** (`jd-graph`). Iteration is expressed with explicit loop nodes, never cycles. The UI offers a linear **stack view** for graphs that are chains (the common artist case) and a full node editor otherwise; both edit the same graph. Ports use a small decomposable type set (Heightfield, Mask, Flow, Color, Vector, Mesh, Material, Instances, Scalar, Vec2/3, Curve) with parameter ports wireable.

## Alternatives rejected
- Layer stack only — ceiling hit by WC; cannot express branching/merging masks.
- Untyped graph — Gaea's hidden type system was called "a deal breaker" by WM users; typed ports enable validation and generated UI.
- Cyclic graphs with fixpoint evaluation — complicates caching/dirty propagation for no artist benefit.

## Consequences
- Evaluation, caching and dirty tracking are standard topological-order algorithms (`04-graph-engine.md`).
- Stack view must degrade gracefully when the graph is not a chain (shows "open in node editor").
- Node kinds are versioned (`kind@version`) with migrations for project longevity.

## Evidence
`docs/research/world-creator.md` §13 "Do differently" 1; `docs/research/gaea-world-machine.md` D1 items 1, 8, 9, 10.
