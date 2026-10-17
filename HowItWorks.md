# How Nodemind Works

Nodemind turns a spoken brain-dump into a clean checklist — and it does the whole thing **on your Mac**. Nothing is uploaded.

## The pipeline

```
🎤 You talk
   │
   ▼
🗣️  Speech → text      Whisper runs locally (whisper.cpp, Metal)
   │
   ▼
🧠 Ramble → structure  A small local LLM (Qwen3.5 0.8B, Metal) cleans the
                       transcript, extracts tasks / topics / people / goals,
                       and links them into your thought graph
   │
   ▼
🗂️  Filed locally       SQLite in your app-data folder: thoughts, tasks,
                       graph, vectors — one file you can inspect
```

## Step by step

1. **Capture** — tap the mic; your audio is recorded at 16 kHz and buffered in-process until you tap again (20-minute cap).
2. **Transcribe** — the recording runs through the local Whisper model. You get raw text, usually in a few seconds on Apple Silicon.
3. **Clean up** — the local LLM rewrites the ramble into crisp checklist items, names the topics / people / goals you mentioned, and (if it sounds like one) recognizes direct voice commands like "remind me to call Sam".
4. **File it** — the thought, its tasks, and its graph links are written to the local database, and a vector of the thought is stored so you can search your memory semantically.
5. **Review** — the result card shows the transcript, a one-line summary, the suggested checklist (tap to keep), and the links it made. The Mind Map and Today views refresh live.

## Where it all lives

- **Engine** — `whisper-cli` and two loopback `llama-server` instances, spawned by the app and killed when it quits.
- **Models** — Whisper small, Qwen3.5 0.8B and EmbeddingGemma 2, downloaded once on first launch into your app-data folder (pinned, checksum-verified).
- **Your data** — `~/Library/Application Support/com.nokast.nodemind/nodemind.db` (openable from Settings).

## Why it stays private

- Speech, reasoning, and embeddings all run on your own Mac — on GPU via **Metal**.
- The only network calls the app can make are the one-time model download (first launch, on slim builds) and the opt-in update check.
- No account, no telemetry, no cloud. Delete the app-data folder and Nodemind has forgotten everything.
