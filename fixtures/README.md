# Fixtures

`malformed/` — a seed corpus of broken heightmap files (empty, magic-only, truncated, absurd
dimensions, offsets past EOF, wrong colour type). `crates/jd-terrain/tests/malformed.rs` feeds every
file to the importer and requires a structured error (`E_FORMAT_CORRUPT` / `E_FORMAT_UNSUPPORTED` /
`E_INPUT_INVALID`) — never a panic, never a silently empty field. Add a file here whenever a real
broken input is found in the wild; the test picks it up by extension. Not malformed, so not here: colour
PNGs (imported as 16-bit luminance by design).

`tiny-grid.fbx` — a 5×5 terrain grid from our own writer (`jd export mesh --step 64` on a 256² world, 8.6 KB): the seed
of the `fbx_mesh` fuzz corpus in CI, so libFuzzer starts past the header instead of from an empty corpus.
