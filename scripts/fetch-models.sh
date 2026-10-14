#!/usr/bin/env bash
# Download the GGUF/ggml models into src-tauri/resources/models/ so the app
# bundle works fully offline out of the box (batteries included).
#
# Resumable (curl -C), checksum-verified, idempotent. ~1.8 GB total; the
# files are already in this repo's git-lfs-less working tree once fetched.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/src-tauri/resources/models"
UA="Nodemind/1.0.0 (bundle prep)"

mkdir -p "$DIR"

fetch() {
  local name="$1" url="$2" sha="$3"
  local dest="$DIR/$name" part="$DIR/.$name.part"

  if [[ -f "$dest" ]]; then
    local got
    got="$(shasum -a 256 "$dest" | awk '{print $1}')"
    if [[ "$got" == "$sha" ]]; then
      echo "✓ $name (already present, checksum ok)"
      return 0
    fi
    echo "✗ $name present but checksum differs — re-downloading"
    rm -f "$dest"
  fi

  echo "▸ $name"
  curl -fL --retry 3 --retry-delay 2 -C - -A "$UA" -o "$part" "$url"
  local got
  got="$(shasum -a 256 "$part" | awk '{print $1}')"
  if [[ "$got" != "$sha" ]]; then
    echo "✗ $name checksum mismatch (got $got, want $sha)" >&2
    rm -f "$part"
    return 1
  fi
  mv "$part" "$dest"
  echo "✓ $name ($(du -h "$dest" | awk '{print $1}'))"
}

# Must match src-tauri/src/models/manifest.rs exactly.
fetch "ggml-small.bin" \
  "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" \
  "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"

fetch "llama-3.2-1b-instruct-q4_k_m.gguf" \
  "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf" \
  "6f85a640a97cf2bf5b8e764087b1e83da0fdb51d7c9fab7d0fece9385611df83"

fetch "embeddinggemma-2-q8_0.gguf" \
  "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/embeddinggemma-2-Q8_0.gguf" \
  "2188ac1deca4b77dffefd603c2776a9d76d9d74ec01841392982ebb840b09135"

# Optional: the whisper CoreML encoder zip (5-10× faster STT on Apple Silicon).
ENC="$DIR/ggml-small-encoder.mlmodelc"
if [[ ! -f "$ENC/model.mil" ]]; then
  ZIP="$DIR/.ggml-small-encoder.mlmodelc.zip.part"
  echo "▸ ggml-small-encoder.mlmodelc (optional CoreML encoder)"
  curl -fL --retry 3 --retry-delay 2 -C - -A "$UA" -o "$ZIP" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-encoder.mlmodelc.zip"
  got="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
  if [[ "$got" != "de43fb9fed471e95c19e60ae67575c2bf09e8fb607016da171b06ddad313988b" ]]; then
    echo "✗ encoder zip checksum mismatch — skipping (STT falls back to CPU)" >&2
    rm -f "$ZIP"
  else
    rm -rf "$ENC"
    unzip -oq "$ZIP" -d "$DIR"
    rm -f "$ZIP"
    echo "✓ ggml-small-encoder.mlmodelc"
  fi
else
  echo "✓ ggml-small-encoder.mlmodelc (already present)"
fi

echo "All models ready in $DIR"
