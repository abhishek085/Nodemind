# Technical Design

## 1. Overview

Nodemind is a **voice-to-checklist web app** that runs entirely in the browser. A user speaks a brain-dump; the app transcribes it and rewrites it into a clean to-do list — with no server, no account, and no data leaving the device.

The whole product is a single static HTML file ([`poc/voice-checklist.html`](poc/voice-checklist.html)) plus a PWA manifest and service worker. It is hosted as static files on GitHub Pages.

## 2. Design principles

1. **On-device first** — all AI runs locally via WebGPU. Privacy is structural, not a policy.
2. **Zero install** — it's a link. No download, no account, no setup beyond a one-time model fetch.
3. **One job, done well** — voice → checklist. No graphs, meetings, or dashboards.
4. **No build step** — plain HTML/CSS/JS so anyone can read, run, and fork it.

## 3. Architecture

```
Browser (everything happens here)
├── UI            vanilla HTML/CSS/JS, state in localStorage
├── Audio         MediaRecorder → 16 kHz mono PCM (Web Audio API)
├── Speech→Text   Whisper (whisper-base) via Transformers.js · WebGPU
├── Cleanup       Llama 3.2 1B via WebLLM (MLC) · WebGPU
└── PWA           Web App Manifest + service worker (offline + installable)
```

There is no backend. Models are downloaded once from a CDN and cached by the libraries (Cache Storage); the service worker caches the app shell and library JS so the app launches offline.

## 4. The cleanup step

This is where the value lives. The transcript is sent to the LLM with a strict system prompt:

- Output JSON only: `{"tasks": [...]}`.
- Include **only** tasks the user actually said — never invent or infer.
- Remove filler, merge duplicates, keep self-corrections, phrase as imperatives.
- If there are no real tasks, return an empty list.

Output is streamed and parsed leniently (extract the first JSON object, fall back to line-splitting).

### Lessons baked in

- **Don't use grammar-constrained JSON mode** (`response_format: json_object`) — it can stall on grammar compilation. Stream plain text and parse instead.
- **Few-shot examples leak** — concrete example tasks were copied verbatim into real output. Use neutral examples + low temperature (0.1) + an explicit "only what they said" rule.
- **Model size** — 0.5B drops tasks; 1.5B is slower; **1B is the sweet spot**.

## 5. Requirements & trade-offs

Running models locally costs broad compatibility:

- Needs **WebGPU** (Chrome/Edge today; Safari partial).
- Best on a **laptop/desktop with ~8 GB+ RAM**.
- A **one-time ~1 GB** download on first use.

This is the deliberate trade for "free + private + no install."

## 6. History

Nodemind began as a native macOS app (Tauri 2 + Rust + `whisper-rs` + Ollama + SQLite) framed as a "cognition agent" with a knowledge graph, meetings, and fog detection. That proved too complex to set up and too abstract for everyday users. The project pivoted to this simpler browser-local tool. The original desktop source lives in the project's git history.

See [HowItWorks.md](HowItWorks.md) for the plain-language version.
