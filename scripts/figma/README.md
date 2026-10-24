# User-guide figures in Figma

The Figma file **JengaDunia-JD** (`fxyeV62hgLoNmG3dU1LX6G`) holds every figure of `docs/user/*.md`
by guide and heading: a Cover page, then **Desktop app**, **CLI quickstart** and **Other**. Each
figure is a card with the image, the guide's alt text, its size and path, and the state of its
annotations. Where the unannotated capture still exists, the red circles, numbered badges
(component `Annotation/Badge`) and captions of `apps/desktop/scripts/annotate.mjs` are rebuilt as
editable layers over it; elsewhere they are part of the PNG. The repository's PNGs stay the
published figures — an annotation edited in Figma is carried back into the figure's
`.annot.json` and re-rendered with `annotate.mjs`.

These scripts built it (2026-10-24) through the Figma MCP server; they are not run by the gates.

1. `python3 scripts/figma/manifest.py <dir>/manifest.json` (from the repository root) lists every
   figure with its guide, heading, alt text, size and annotation spec.
2. `python3 scripts/figma/build_pages.py <dir>` writes one Plugin API script per page
   (`page_desktop.js`, `page_cli.js`, `page_other.js`) for the `use_figma` tool. Page, style and
   component ids are those of the file above; a new file needs new ids (`use_figma` returns them
   when the styles and the badge component are created).
3. `upload_assets` with the image frames' ids returns one upload URL per frame; list them as
   `<node> <uuid>` lines in `<dir>/urls_<page>.txt`, write `<dir>/upload_<page>.json` (node, file,
   source path) and run `python3 scripts/figma/post_uploads.py <dir> <page> <page id>`, which
   checks that every image landed on its frame.
