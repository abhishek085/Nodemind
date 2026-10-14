# Technical Design

Nodemind is a **voice-first personal thinking assistant** for macOS. It is a native
Tauri 2 app: speak a brain-dump (English or Hinglish), and a fully local pipeline
transcribes it, cleans it, extracts the structure (tasks, topics, people, goals),
files it into a thought graph and a vector store, and surfaces it in three views —
**Capture**, **Mind Map**, and **Today**.

Nothing leaves the machine. There is no backend, no account, no telemetry.

## 1. Design principles

1. **Local-first is structural, not a policy.** STT, LLM, and embeddings are bundled
   sidecars + models. The app's *only* network uses are the one-time model download
   (first launch) and the opt-in update check.
2. **Voice is the input; dashboards are the output.** The UX is never a chat window.
   You talk; the app organizes and displays.
3. **One job per module.** Audio, STT, LLM logic, state/memory, and UI are hard
   boundaries. The Rust backend owns all AI; the frontend is a thin renderer over IPC.
4. **Everything is inspectable.** SQLite, not blobs: thoughts, tasks, graph nodes and
   edges, vectors, focus sessions, and calendar snapshots are plain rows in one file.
5. **Graceful degradation.** A missing calendar permission, a failed LLM call, or a
   silent recording degrades to a smaller feature — never a crash.

## 2. Architecture

```
┌──────────────────────────────  Tauri webview (React 19 + Tailwind)  ──────────────────────────────┐
│  CaptureView        GraphView (React Flow)     TodayView          SettingsView      SetupScreen   │
│  Web Audio worklet  search + node detail       brief / focus /    engine + models   model         │
│  16 kHz s16le  →    ←───────── get_graph ────  checklist /        language / data   download      │
│  base64 chunks      ←────── get_thoughts ───  calendar / tasks   reindex / updates progress      │
└──────────────┬──────────────────────────────────────────────────────────────────────┬─────────────┘
               │  invoke (IPC)                                                        │ events
┌──────────────▼──────────────────────────────────────────────────────────────────────▼─────────────┐
│  Rust (src-tauri/src/)                                                                              │
│                                                                                                    │
│  commands/         the IPC surface: capture · thoughts · today · calendar · settings · updates ·   │
│  (thin)            engine — validate, lock state, emit events, return plain serializable shapes    │
│                                                                                                    │
│  engine/           pipeline.rs   capture → wav → whisper → clean → annotate → embed → store        │
│                    sidecar.rs    spawns/health-checks two loopback llama-servers; killed on exit   │
│                    whisper.rs    whisper-cli per transcription (Metal, auto CoreML encoder)        │
│                    llm.rs        chat() + extract_json(): clean · annotate · fog · brief ·         │
│                                  parse_command · embed()   — all prompts from resources/prompts.toml│
│  store/            db.rs         SQLite: thoughts · tasks · calendar_day · focus · meta            │
│                    graph.rs      graph_nodes / graph_edges + fuzzy label merge (cosine ≥ 0.8)      │
│                    vec_.rs       brute-force cosine kNN over f32 BLOBs (per-row model + dim)       │
│  models/           manifest      pinned URLs + sha256 + expected sizes                              │
│                    downloader    resumable, streaming, progress events                             │
│  calendar/         osascript → Apple Calendar events (read-only); errors → permission_hint         │
│  state.rs          Arc<AppState>: db conn, sidecar children, capture buffer, engine status        │
└──────────────┬──────────────────────────────────────┬─────────────────────────────────────────────┘
               │ spawn (127.0.0.1 only)              │ reads/writes
   ┌───────────▼───────────┐              ┌──────────▼──────────────────────────────┐
   │ sidecars (bundled)    │              │ app-data: ~/Library/Application Support  │
   │ llama-server :8642    │              │   /com.nokast.nodemind/                  │
   │   → Llama 3.2 1B      │              │   nodemind.db · models/ (whisper ggml-   │
   │ llama-server :8643    │              │   small + CoreML encoder, Llama 1B,      │
   │   → EmbeddingGemma 2  │              │   EmbeddingGemma 2) + prompts.toml copy │
   │ whisper-cli (on demand)│             └──────────────────────────────────────────┘
   └───────────────────────┘
```

### Why two loopback servers

llama.cpp's `llama-server` keeps the model resident and serves OpenAI-compatible
`/v1/chat/completions` and `/v1/embeddings`. One instance per model (ports 8642 chat,
8643 embed) means zero model (re)load cost between calls. They bind to `127.0.0.1`,
are spawned by the app at boot, health-checked, and killed on exit.

## 3. The capture pipeline

```
PCM (webview, 16 kHz mono s16le, base64 chunks → append_audio)
        │  (Rust buffers into a wav; 20-minute cap; silence & no-audio fast paths)
        ▼
whisper-cli  ── ggml-small (+ CoreML encoder when present) on Metal ──▶ transcript
        │
        ├─ clean_checklist()   LLM: ramble → 3–6 crisp checklist items
        ├─ annotate()          LLM: summary · tasks · topics · people · goals · related · sentiment
        ├─ parse_command()     LLM, only if the text *looks* like a direct command
        │                      (cheap regex pre-filter): create_task / start_focus
        ├─ fog()               LLM, only for longer transcripts: friction signals → meta
        └─ embed()             EmbeddingGemma 2 vectors:
                │                  • thought vector  → vectors table (ref = thought id)
                │                  • label vectors   → graph node vectors (fuzzy merge key)
                ▼
SQLite:  thoughts row (+ backfilled summary) · tasks from annotation & command ·
         graph nodes/edges (thought node keyed by UUID; entities by label slug;
         fuzzy merge re-attaches near-duplicate labels, cosine ≥ 0.8) ·
         calendar/focus/meta as needed
        ▼
events:  capture-stage → capture-complete → UI result card (checklist to keep,
         chips, command actions) · graph + today views refresh
```

Every LLM step tolerates failure: a missing model or a malformed JSON answer yields
an empty structure, never an error, so a capture always produces *something* useful.

## 4. Data model (SQLite)

| table | purpose |
| --- | --- |
| `thoughts` | one per capture: text, summary, language, day, started_at, duration |
| `tasks` | checklist items (from captures or manual), optional project, done flag |
| `graph_nodes` | `id · node_type(thought\|topic\|person\|goal\|…) · label · mentions · last_seen · sentiment` — thought nodes are keyed by the thought UUID; entities by a label slug |
| `graph_edges` | `from · to · edge_type(involves\|mentions\|supports\|relates_to) · weight` — upserted, weight strengthens with repetition |
| `vectors` | `kind · ref_id · model · dim · f32 BLOB` — brute-force cosine kNN (fine at personal scale) |
| `calendar_day` | last osascript snapshot per day (read-only mirror) |
| `focus_sessions` | start/end, minutes |
| `meta` | key/value: app version, fog signal slice, counters |

## 5. Prompts

All prompts live in `resources/prompts.toml` (bundled; a user-editable copy is made
in app-data and preferred at runtime). Each prompt is a named section with
`system`/`user` templates, `{placeholder}` substitution, and explicit "ONLY valid
JSON" instructions; Rust parses defensively (`extract_json` tolerates code fences and
stray prose). Temperatures are deliberately low (0.1–0.4).

## 6. The three views

- **Capture** — one mic button → recording (timer) → "transcribing… organizing…" →
  result card: transcript, one-line summary, *suggested checklist* (tap to keep),
  topic/people chips, and direct-command actions (e.g. start the parsed focus block).
- **Mind Map** — React Flow. Thoughts on an inner ring; topics/people/goals on the
  outer ring in type sectors. Node size = degree + mentions. Search runs local
  vector kNN over the embed model; node click opens a detail panel.
- **Today** — a `daily_brief` LLM insight (friction + one tiny next step), the
  checklist, a 25-minute focus block with a live countdown, and the day's calendar
  (with a permission hint when AppleScript can't read it).

## 7. Setup & update flow

1. **First launch** — `begin_setup` downloads any missing models (pinned sha256,
   resumable, streamed progress) into app-data, then boots the engine. The bundled
   `resources/models/` makes this a no-op for DMG installs; the download path exists
   for slim builds and model upgrades.
2. **Each launch** — models are resolved *app-data first, bundle second*, the two
   servers boot on loopback, and the tray icon tracks engine health.
3. **Updates** — the updater plugin checks the GitHub release endpoint against the
   embedded public key, downloads the signed `.app.tar.gz`, replaces the bundle,
   relaunches from `current_exe()`, and exits.

## 8. Build & distribution

- **Sidecars** — `scripts/build-sidecars.sh`: pinned `whisper.cpp v1.9.5` +
  `llama.cpp b11609`, Metal on, **statically linked** (`BUILD_SHARED_LIBS=OFF`,
  `LLAMA_OPENSSL=OFF`) so each is a single self-contained binary under
  `src-tauri/binaries/` (git-ignored; Tauri `externalBin`).
- **Models** — `scripts/fetch-models.sh`: pinned HF URLs, resume + sha256 verification,
  into `src-tauri/resources/models/` (git-ignored; bundled via `bundle.resources`).
- **App** — `npx tauri build` → `Nodemind.app` + **DMG** with everything bundled
  (~1.9 GB, fully offline). Release signing uses `scripts/release.sh`
  (`tauri signer sign` + `gh release create`).
- **MacOS specifics** — `Info.plist` carries `NSMicrophoneUsageDescription`
  (system prompt on first capture); window is an overlay title bar; tray icon with
  left-click-to-open.

## 9. What is deliberately *not* in scope

Cloud sync, multi-user, chat UX, advanced graph layouts/analytics, iOS, and any
feature that would require the user's data to leave the machine. (See AGENTS.md.)
