# 2026-10-21 — research follow-ups closed

Evidence for the follow-up list in `evidence-register.md` (items 1, 3, 4, 5, 6, 7). Method: page fetches (summarised by a reader model, quotes checked where given), a real browser for the JS site, local tools on real exports.

| # | Source | Finding |
|---|--------|---------|
| 1 | `github.com/microsoft/MoGe` README · `huggingface.co/Ruicheng/moge-2-vitl-normal` card | Code MIT (DINOv2 part Apache 2.0); weights licence tag `mit`, README empty, no restriction. |
| 1 | `github.com/ByteDance-Seed/Depth-Anything-3` README model table | Code Apache-2.0. Weights: Small, Base, Metric-Large, Mono-Large = Apache 2.0; Large(-1.1), Giant(-1.1), Nested = CC BY-NC 4.0. |
| 3 | `usdchecker --arkit` (Apple USD Tools 0.25.11, `/usr/bin`) on `/tmp/jd-accept/Acceptance.jdproj/exports/terrain.usdz` and a fresh 64² export | "Success!" both; zip entry stored, data offset 64. Bug fixed: entry named by the given path → bare name. |
| 4 | `integrations/unity/Editor/JengaDuniaImport.cs` lines 97–114; Unity manual (Import Raw: Byte Order Windows/Mac) | Our importer reads LE bytes itself; the dialog toggle is irrelevant to the profile. Verified headlessly by the acceptance item 13. |
| 5 | Wayback copy of `FastErosion_PG07.pdf` (10 pp.), text via PDFKit | Equations (2), (4), (7), (8–9), (10), (11–12) match `crates/jd-terrain/src/sim.rs`; transport is donor-cell, not eq. (13). |
| 6 | READMEs of `weigert/SimpleHydrology`, `SimpleErosion`, `SoilMachine` | "MIT License" each. |
| 7 | `procedural-terrains.com` in Chromium | JS app, no terms/licence text. |

Still open: 2 (WebView2 / WebKitGTK — no Windows or Linux machine here) and 8 (the MoGe-2 / DA3 benchmark, to run with `jd-vision`).
