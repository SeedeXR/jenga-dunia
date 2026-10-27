#!/usr/bin/env bash
# Build, test and install Jenga Dunia on this Mac, signed without an Apple Developer account.
#
#   scripts/install.sh                 # every test, then build, sign and install to /Applications
#   scripts/install.sh --skip-tests    # build, sign and install only
#   scripts/install.sh --no-install    # every test, build and sign; leave the app in target/release
#   scripts/install.sh --dest ~/Apps   # install somewhere else
#
# What "self notarization" can and cannot be: Apple's notarization is Apple's server checking a
# Developer ID signature, so it needs a paid Apple Developer account and nothing local replaces
# it. Without one this does what a Mac allows on its own:
#   1. scripts/package.sh signs the bundle ad hoc, the Editor helper first, so it is sealed and
#      `codesign --verify` passes (an unsealed bundle reads as "damaged");
#   2. the install removes the quarantine flag, so this Mac opens the app without the Gatekeeper
#      prompt (a locally built app has none; a copied or downloaded one does);
#   3. it registers the app with Launch Services, so .jdproj files open with it.
# Gatekeeper's own assessment (`spctl`) still rejects an ad-hoc app — that verdict needs Developer
# ID + notarization — and a copy taken to another Mac needs right-click ▸ Open there once.
#
# Needs: the Xcode command line tools, Rust, Node + pnpm; cargo-packager 0.11.8 and Playwright's
# Chromium are installed if missing.
set -euo pipefail

tests=1
install=1
dest=""
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-tests) tests=0 ;;
    --no-install) install=0 ;;
    --dest) dest="${2:?--dest needs a directory}"; shift ;;
    -h | --help) awk 'NR > 1 { if (!/^#/) exit; sub(/^# ?/, ""); print }' "$0"; exit 0 ;;
    *) echo "install: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done
# A relative --dest is the caller's, not the repo's (the cd below).
case "$dest" in "" | /*) ;; *) dest="$PWD/$dest" ;; esac
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }
[ "$(uname)" = Darwin ] || { echo "install: macOS only" >&2; exit 1; }

step "Prerequisites"
xcode-select -p >/dev/null 2>&1 || { echo "install: the Xcode command line tools are required — xcode-select --install" >&2; exit 1; }
if ! cargo packager --version 2>/dev/null | grep -q '0\.11\.'; then
  cargo install cargo-packager --version 0.11.8 --locked
fi
(cd apps/desktop && pnpm install --frozen-lockfile)

if [ "$tests" = 1 ]; then
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' EXIT
  step "Tests: Rust (cargo test --workspace)"
  cargo test --workspace
  cargo build -q -p jd-cli -p jd-mcp
  step "Tests: the Editor (types and build, unit, end to end against the real core)"
  (cd apps/desktop && pnpm build && pnpm test && pnpm exec playwright install chromium && pnpm test:e2e)
  step "Tests: the acceptance scenario (brief §20, --quick)"
  node scripts/acceptance.mjs --quick --out "$scratch/acceptance" --report "$scratch/acceptance.md"
  grep '^Summary:' "$scratch/acceptance.md"
fi

step "Build and sign: release binaries, the app bundle (scripts/package.sh)"
scripts/package.sh -f app
app="target/release/Jenga Dunia.app"
codesign --verify --deep --strict "$app"

if [ "$install" = 0 ]; then
  step "Done (not installed): $app"
  exit 0
fi

step "Install"
if [ -z "$dest" ]; then
  if [ -w /Applications ]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi
mkdir -p "$dest"
target="$dest/Jenga Dunia.app"
# Never replace the bundle under a running Studio (`tell … to quit` would launch a stopped one).
if pgrep -x jd-studio >/dev/null; then
  echo "quitting the running Jenga Dunia"
  osascript -e 'tell application "Jenga Dunia" to quit' >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do pgrep -x jd-studio >/dev/null || break; sleep 0.5; done
  if pgrep -x jd-studio >/dev/null; then
    echo "install: Jenga Dunia is still running (a save prompt?) — quit it and run again" >&2
    exit 1
  fi
fi
rm -rf "$target"
ditto "$app" "$target"
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$lsregister" -f "$target"
# The build copy has the same bundle id: unregistered, .jdproj files open the installed app.
"$lsregister" -u "$app" 2>/dev/null || true

step "Gatekeeper's view (informational)"
# Expected without a Developer ID: "rejected". This Mac still opens the app (no quarantine flag).
spctl --assess --type execute -v "$target" 2>&1 || true

step "Installed: $target"
echo "Open it with:  open \"$target\"   (or double-click a .jdproj)"
