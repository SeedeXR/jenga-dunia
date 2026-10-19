# Architecture Decision Records

Format: Status · Date · Context · Decision · Alternatives rejected (and why) · Consequences · Evidence. One decision per file; supersede rather than edit history.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-one-core-three-adapters.md) | One Rust core, thin GUI/CLI/MCP adapters | Accepted |
| [0002](0002-dag-graph-with-stack-view.md) | DAG node graph as source of truth; stack view as projection | Accepted |
| [0003](0003-viewport-hosting.md) | Viewport hosting strategy | Proposed (prototype gate) |
| [0004](0004-frontend-stack.md) | Frontend framework and node-editor library | Proposed |
| [0005](0005-f32-tiles-with-halos.md) | f32 heightfield tiles with halos as the unit of work | Accepted |
| [0006](0006-erosion-implementation-order.md) | Erosion: virtual pipes first, droplet second, stream power later | Accepted |
| [0007](0007-determinism-contract.md) | Determinism contract and CPU/GPU tolerance tiers | Accepted |
| [0008](0008-performance-methodology.md) | Measured performance: benches, peak RSS gate, 4 GB ceiling | Accepted |
| [0009](0009-out-of-process-plugins.md) | Out-of-process plugins for licence and crash isolation | Accepted |
| [0010](0010-rea-and-clean-room-policy.md) | REA usage and clean-room policy | Accepted |
| [0011](0011-project-as-directory.md) | Project is a directory with versioned JSON + disposable cache | Accepted |
| [0012](0012-format-strategy.md) | Format strategy: glTF + USD pure Rust first, FBX via adapter | Proposed |
