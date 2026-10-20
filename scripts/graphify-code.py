#!/usr/bin/env python3
"""Deterministic code knowledge graph of the repository: graphify's AST extraction only (no LLM,
no API key) → graphify-out/graph.json + GRAPH_REPORT.md. The session-end checklist and CI run it;
`--check` builds twice and fails when the two graphs differ (nodes, edges, communities).

Usage: <graphify python> scripts/graphify-code.py [root] [--check]
       (CI: pip install graphifyy==0.9.13; locally the interpreter behind `which graphify`)
"""
import json
import subprocess
import sys
from pathlib import Path

from graphify.analyze import god_nodes, surprising_connections, suggest_questions
from graphify.build import build_from_json
from graphify.cluster import cluster, score_all
from graphify.detect import detect
from graphify.export import to_json
from graphify.extract import collect_files, extract
from graphify.report import generate

args = [a for a in sys.argv[1:] if not a.startswith("--")]
root = Path(args[0] if args else ".").resolve()
out = root / "graphify-out"
out.mkdir(exist_ok=True)


def build() -> tuple[dict, int, int, int]:
    detection = detect(root, extra_excludes=["target", "node_modules", "graphify-out", "installables", "dist"])
    code_files: list[Path] = []
    for f in detection.get("files", {}).get("code", []):
        p = Path(f)
        code_files.extend(collect_files(p, root=root) if p.is_dir() else [p])
    extraction = extract(sorted(set(code_files)), cache_root=root, parallel=False)
    G = build_from_json(extraction, root=root)
    if G.number_of_nodes() == 0:
        sys.exit("graphify: extraction produced no nodes")
    communities = cluster(G)
    graph = {
        "nodes": sorted(G.nodes()),
        "edges": sorted((min(a, b), max(a, b)) for a, b in G.edges()),
        "communities": sorted(sorted(m) for m in communities.values()),
    }
    return (graph, G, communities, extraction, detection), G.number_of_nodes(), G.number_of_edges(), len(communities)


(first, G, communities, extraction, detection), n, e, c = build()
print(f"graphify: {n} nodes, {e} edges, {c} communities (AST only)")
if "--check" in sys.argv:
    (second, *_), *_ = build()
    if first != second:
        sys.exit("graphify: two builds differ — the graph is not deterministic")
    print("graphify: second build identical")

commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, cwd=root).stdout.strip() or None
cohesion = score_all(G, communities)
labels = {cid: f"Community {cid}" for cid in communities}
gods = god_nodes(G)
surprises = surprising_connections(G, communities)
questions = suggest_questions(G, communities, labels)
to_json(G, communities, str(out / "graph.json"), force=True, built_at_commit=commit, community_labels=labels)
tokens = {"input": 0, "output": 0}
(out / "GRAPH_REPORT.md").write_text(
    generate(G, communities, cohesion, labels, gods, surprises, detection, tokens, str(root), questions, built_at_commit=commit),
    encoding="utf-8",
)
print(f"graphify: wrote {out / 'graph.json'} and GRAPH_REPORT.md (commit {commit})")
