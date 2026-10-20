#!/usr/bin/env bash
# Every gate CI runs, in one go. Run before `git push`; stops at the first failure.
# Usage: git add -A && scripts/pre-push.sh   (from the repo root; ~2–3 min warm — the freshness checks diff against the index)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo run -q -p jd-cli -- schema --write schemas/v1 >/dev/null
git diff --exit-code -- schemas/ || { echo "schemas/v1 regenerated — commit it"; exit 1; }
node scripts/check-version.mjs
node scripts/check-links.mjs
( cd apps/desktop && pnpm types:gen >/dev/null && git diff --exit-code -- src/lib/ops.gen.ts || { echo "ops.gen.ts regenerated — commit it"; exit 1; } )
( cd apps/desktop && pnpm build && pnpm test && pnpm test:e2e )
echo "all gates green"
