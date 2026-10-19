# Session Start — Mandatory Onboarding

Every agent (and every human contributor using an agent) performs these steps **before** reading or writing any code. Do not skip steps; do not reorder.

## 1. Read, in this order

1. `memory/agent_profile.md` — who you are, the zero-hallucination and testing standards.
2. `memory/project_context.md` — purpose, deliverables, success criteria.
3. `memory/philosophy.md` — engineering philosophy and platform stance.
4. `memory/handover_session.md` — read the **latest** session entry only, then skim earlier ones if the latest refers to them.
5. `memory/todo.md` — current backlog, ownership tags, what is in progress.
6. `memory/mindmap.md` — the system map; locate the subsystem you will touch.
7. `memory/instruction.md` — implementation guidance for that subsystem.
8. `docs/README.md` — index to the single source of truth (architecture, ADRs, specs, research).
9. Only if the task concerns reference tools / porting behaviour: `memory/porting.md`.

## 2. Reconcile memory with live state

Memory is point-in-time; the repository is truth.

```bash
git status && git log --oneline -15
cargo --version && node --version && pnpm --version      # toolchain present?
ls docs/adr | tail -5                                    # any ADRs newer than the handover?
cargo test --workspace 2>&1 | tail -20                   # once a workspace exists: what is red right now?
```

If the handover claims something the repository contradicts, the repository wins; note the discrepancy in your session entry.

## 3. Operational mindset

- Ground → comprehend → choose → execute → verify → report. In that order.
- Read the code the task touches end-to-end before editing. Trace callers and data flow.
- Fix root causes in the shared path, not symptoms at one call site.
- Climb the ladder before writing code: needed at all? already in the codebase? stdlib? platform-native? installed dependency? one line? only then minimal new code.
- Long-running work lives on paper: keep `todo.md` and your handover entry current so a fresh context can resume.

## 4. Token-efficiency standards

- Batch independent tool calls in one response.
- Delegate wide reads/research to sub-agents; keep decisions in the main thread; spot-check load-bearing claims.
- Do not paste large files into chat; cite `path:line`.
- Write deliverables once; edit surgically.

## 5. Zero-hallucination expectations

- Observed vs inferred vs assumed — label them. Cite `file:line` or the command and its real output.
- No invented versions, crate names, licences, APIs or test results. "Could not verify" is an acceptable and required answer.
- A gate you cannot pass honestly (test fails, SDK unavailable, validator missing) is reported as such.

## 6. Before you finish a session

1. Run the gates relevant to what you touched (see testing table in `agent_profile.md`).
2. Run `/code-review` and `/ponytail-review` on the diff; fix findings.
3. Append a dated entry to `memory/handover_session.md` (format in that file).
4. Update `memory/todo.md` (check off, strike through with reason, add new).
5. Update `memory/mindmap.md` if structure changed; add/update an ADR if a decision was made.
6. Regenerate the knowledge graph if the project structure changed (`/graphify` → `graphify-out/`), keep it deterministic.
7. Commit with a conventional message; push only after the review loop passed.
