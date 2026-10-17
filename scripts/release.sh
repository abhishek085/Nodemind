#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Sign & publish a NodeMind release for the in-app updater.
#
#   ./scripts/release.sh v1.0.1   [release-notes...]
#
# Artifacts produced (by `tauri build`, with a signing key present):
#   • Nodemind_1.0.1_aarch64.dmg        — for humans (drag-to-Applications)
#   • Nodemind-macOS.dmg                — same DMG under a stable name, so
#                                         the website's .../releases/latest/
#                                         download/Nodemind-macOS.dmg link
#                                         always serves the newest build
#   • Nodemind.app.tar.gz + .sig        — for the Tauri updater
#   • nodemind-updater.json             — static update manifest (what the
#                                         app's updater endpoint fetches)
# All four are attached to a GitHub Release on abhishek085/Nodemind.
#
# Requires:
#   • src-tauri/nodemind.key            (updater private key; git-ignored)
#   • gh CLI, authenticated (gh auth status)
#   • models + sidecars present in src-tauri (see scripts/fetch + build)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${1:?usage: ./scripts/release.sh vX.Y.Z [notes...]}"
shift || true
NOTES="${*:-See the repo for changes.}"

export TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-$PWD/src-tauri/nodemind.key}"
[ -f "$TAURI_SIGNING_PRIVATE_KEY" ] || { echo "missing updater key: $TAURI_SIGNING_PRIVATE_KEY"; exit 1; }
command -v gh >/dev/null || { echo "gh CLI not found — install it and run: gh auth login"; exit 1; }

APP_NAME="Nodemind"
BUNDLE="src-tauri/target/release/bundle/macos"
VERSION="${TAG#v}"

echo "▸ building app + bundles"
npx tauri build >/dev/null

DMG=$(find src-tauri/target/release/bundle/dmg -name "${APP_NAME}_*.dmg" | head -1)
TARBALL="$BUNDLE/${APP_NAME}.app.tar.gz"
SIG="$TARBALL.sig"
[ -n "$DMG" ] || { echo "no DMG found — did the build succeed?"; exit 1; }
[ -f "$TARBALL" ] || { echo "missing updater artifact: $TARBALL (is createUpdaterArtifacts on + key set?)"; exit 1; }

STABLE_DMG="$(dirname "$DMG")/${APP_NAME}-macOS.dmg"
cp "$DMG" "$STABLE_DMG"

# `tauri build` already signed the updater tarball (TAURI_SIGNING_PRIVATE_KEY).
echo "▸ checking updater signature"
[ -f "$SIG" ] || { echo "signing did not produce $SIG"; exit 1; }

REPO="abhishek085/Nodemind"
MANIFEST="$BUNDLE/nodemind-updater.json"
python3 - "$VERSION" "$TAG" "$SIG" "$MANIFEST" "$NOTES" "$REPO" "$APP_NAME" <<'PY'
import json, sys, datetime
version, tag, sig_path, out, notes, repo, app_name = sys.argv[1:8]
sig = open(sig_path).read().strip()
doc = {
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "platforms": {
        "darwin-aarch64": {
            "url": f"https://github.com/{repo}/releases/download/{tag}/{app_name}.app.tar.gz",
            "signature": sig,
        }
    },
}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(f"manifest → {out}")
PY

echo "▸ publishing $TAG"
gh release create "$TAG" \
  --title "Nodemind $TAG" \
  --notes "$NOTES" \
  "$DMG" "$STABLE_DMG" "$TARBALL" "$SIG" "$MANIFEST"

echo "✓ published: https://github.com/$REPO/releases/tags/$TAG"
echo "  installed apps will pick up the next version via nodemind-updater.json"
