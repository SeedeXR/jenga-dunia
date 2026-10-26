#!/usr/bin/env bash
# Package Jenga Dunia (ADR-0017 P5): Studio is the entry point, the Editor (jd-app) ships beside it.
# Builds both release binaries (the Editor with its web UI embedded), then `cargo packager`
# (config: [package.metadata.packager] in apps/studio/Cargo.toml). Output: target/release/
# (macOS: "Jenga Dunia.app" and a .dmg). Extra arguments go to cargo packager, e.g. `-f app`.
# Needs: pnpm, cargo-packager 0.11 (`cargo install cargo-packager --version 0.11.8 --locked`).
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo-packager >/dev/null || {
  echo "package: cargo-packager is not installed (cargo install cargo-packager --version 0.11.8 --locked)" >&2
  exit 1
}
(cd apps/desktop && pnpm exec tauri build --no-bundle)
cargo build --release -p jd-studio
if [ "$(uname)" = Darwin ]; then
  # The Editor as a helper app (embedded by cargo packager): its own bundle id and no document
  # types, so Finder opens and quit go to Studio, the bundle's main executable.
  helper="target/release/Jenga Dunia Editor.app/Contents"
  rm -rf "target/release/Jenga Dunia Editor.app"
  mkdir -p "$helper/MacOS" "$helper/Resources"
  cp target/release/jd-app "$helper/MacOS/"
  cp apps/desktop/src-tauri/icons/icon.icns apps/desktop/src-tauri/icons/Assets.car "$helper/Resources/"
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
  cat > "$helper/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>jd-app</string>
  <key>CFBundleIdentifier</key><string>com.seedexr.jengadunia.editor</string>
  <key>CFBundleName</key><string>Jenga Dunia Editor</string>
  <key>CFBundleDisplayName</key><string>Jenga Dunia Editor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleIconFile</key><string>icon.icns</string>
  <key>CFBundleIconName</key><string>icon</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
fi
cargo packager --release -p jd-studio "$@"
