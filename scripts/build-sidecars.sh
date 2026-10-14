#!/usr/bin/env bash
# Build NodeMind's local AI sidecars from source and install them as Tauri
# external binaries into src-tauri/binaries/<name>-<triple>.
#
#   whisper.cpp  v1.9.5   → whisper-cli   (Metal/CoreML on Apple Silicon)
#   llama.cpp    b11609   → llama-server  (Metal)
#
# One-time cost (~10–20 min on Apple Silicon). Re-run after bumping the
# pinned versions below. Requires: git, cmake, a C/C++ toolchain.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$ROOT/.sidecar-builds"
OUT="$ROOT/src-tauri/binaries"
TRIPLE="$(rustc -vV | awk -F ' ' '/host/ {print $2}')"
JOBS="$(sysctl -n hw.ncpu 2>/dev/null || nproc)"

WHISPER_REF="v1.9.5"
LLAMA_REF="b11609"

mkdir -p "$OUT" "$BUILD"

# ── whisper.cpp ─────────────────────────────────────────────────────────────
if [[ ! -d "$BUILD/whisper.cpp" ]]; then
  echo "▸ cloning whisper.cpp $WHISPER_REF"
  git clone --branch "$WHISPER_REF" --depth 1 https://github.com/ggml-org/whisper.cpp "$BUILD/whisper.cpp"
fi
cmake -S "$BUILD/whisper.cpp" -B "$BUILD/whisper.cpp/build" \
  -DCMAKE_BUILD_TYPE=Release -DGGML_METAL=ON -DBUILD_SHARED_LIBS=OFF
cmake --build "$BUILD/whisper.cpp/build" --target whisper-cli -j "$JOBS"
install -m 755 "$BUILD/whisper.cpp/build/bin/whisper-cli" "$OUT/whisper-cli-$TRIPLE"
echo "✓ whisper-cli → $OUT/whisper-cli-$TRIPLE"

# ── llama.cpp ───────────────────────────────────────────────────────────────
if [[ ! -d "$BUILD/llama.cpp" ]]; then
  echo "▸ cloning llama.cpp $LLAMA_REF"
  git clone --branch "$LLAMA_REF" --depth 1 https://github.com/ggml-org/llama.cpp "$BUILD/llama.cpp"
fi
cmake -S "$BUILD/llama.cpp" -B "$BUILD/llama.cpp/build" \
  -DCMAKE_BUILD_TYPE=Release -DGGML_METAL=ON -DBUILD_SHARED_LIBS=OFF \
  -DLLAMA_OPENSSL=OFF
cmake --build "$BUILD/llama.cpp/build" --target llama-server -j "$JOBS"
install -m 755 "$BUILD/llama.cpp/build/bin/llama-server" "$OUT/llama-server-$TRIPLE"
echo "✓ llama-server → $OUT/llama-server-$TRIPLE"

echo "All sidecars built. Run the app to smoke-test them."
