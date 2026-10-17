#!/usr/bin/env bash
# Install the latest Nodemind into /Applications.
#
#   curl -fsSL https://raw.githubusercontent.com/abhishek085/Nodemind/main/scripts/install.sh | bash
#
# Why a script: Nodemind isn't notarized by Apple yet, so macOS blocks an app
# downloaded in a browser. A download made with curl isn't flagged that way,
# so this installs with no warning. Read it before you run it — it only
# downloads the release DMG from GitHub and copies the app.
set -euo pipefail

URL="https://github.com/abhishek085/Nodemind/releases/latest/download/Nodemind-macOS.dmg"
[ "$(uname -s)" = "Darwin" ] || { echo "Nodemind is for macOS."; exit 1; }
[ "$(uname -m)" = "arm64" ] || { echo "Nodemind needs an Apple Silicon Mac (M1 or newer)."; exit 1; }

TMP="$(mktemp -d)"
MNT="$TMP/mnt"
cleanup() { hdiutil detach "$MNT" -quiet 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

echo "▸ downloading Nodemind (about 20 MB)…"
curl -fL --progress-bar -o "$TMP/Nodemind.dmg" "$URL"

echo "▸ installing to /Applications…"
mkdir -p "$MNT"
hdiutil attach "$TMP/Nodemind.dmg" -nobrowse -readonly -mountpoint "$MNT" -quiet
pkill -x nodemind 2>/dev/null || true
rm -rf /Applications/Nodemind.app
cp -R "$MNT/Nodemind.app" /Applications/
xattr -dr com.apple.quarantine /Applications/Nodemind.app 2>/dev/null || true

echo "✓ Installed. Opening Nodemind — the first launch downloads about 1.3 GB of AI models."
open /Applications/Nodemind.app
