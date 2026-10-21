#!/bin/sh
# Regenerates the desktop app's icon set from the brand artwork (run from apps/desktop):
#   src-tauri/icons/app-icon.svg  → the legacy set (png sizes, icon.ico, Square logos)
#                                   via `tauri icon`: the mark on the brand gradient, pre-cut to the
#                                   macOS icon grid, for macOS 15 and earlier, Windows and Linux.
#   src-tauri/icons/icon.icon      → Resources/Assets.car via Xcode's actool: the Icon Composer
#                                   document macOS 26+ draws (the system supplies the glass shape,
#                                   so a legacy .icns would be shrunk onto a grey plate there).
# Needs Xcode 26+ for actool; the catalog is committed so a build without Xcode still bundles it.
set -e
cd "$(dirname "$0")/.."
pnpm exec tauri icon src-tauri/icons/app-icon.svg
rm -rf src-tauri/icons/android src-tauri/icons/ios
tmp=$(mktemp -d)
xcrun actool src-tauri/icons/icon.icon --compile "$tmp" --app-icon icon --output-partial-info-plist "$tmp/partial.plist" \
  --minimum-deployment-target 10.15 --platform macosx --target-device mac --include-all-app-icons \
  --standalone-icon-behavior all --errors --warnings --notices --output-format human-readable-text
cp "$tmp/Assets.car" src-tauri/icons/Assets.car
cp "$tmp/icon.icns" src-tauri/icons/icon.icns # actool also renders the legacy icns on Apple's exact grid
rm -rf "$tmp"
echo "icons regenerated; Assets.car $(stat -f %z src-tauri/icons/Assets.car) bytes"
