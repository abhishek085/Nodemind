<p align="center">
  <img src="public/nodemind_icon.png" alt="Nodemind icon" width="110" />
</p>

<h1 align="center">Nodemind</h1>

<p align="center">
  <em>Talk out your messy thoughts. Get a clean mind.</em>
</p>

<p align="center">
  A voice-first, fully local <strong>personal thinking assistant</strong> for macOS.<br />
  Speak — in English or Hindi — and Nodemind turns the ramble into a clean checklist,
  a living thought graph, and a friction-free plan for today.
</p>

<p align="center">
  <img alt="Platform" src="https://img.shields.io/badge/platform-macOS-6366f1?style=flat-square&logo=apple" />
  <img alt="Privacy" src="https://img.shields.io/badge/privacy-100%25_offline-34d399?style=flat-square" />
  <img alt="Engine" src="https://img.shields.io/badge/engine-Whisper_+_Llama_3.2-818cf8?style=flat-square" />
  <img alt="Stack" src="https://img.shields.io/badge/stack-Tauri_2_+_React_fbbf24?style=flat-square" />
  <img alt="License" src="https://img.shields.io/badge/license-MIT-111111?style=flat-square" />
</p>

<p align="center">
  <sub>a <strong>Nokast</strong> project</sub>
</p>

---

## 💡 The idea

You have too many half-formed ideas, tasks, and worries bouncing around your head.
Nodemind gives them a place to land:

1. **🎤 Capture** — tap the mic and brain-dump. Messy is fine. English or Hinglish.
2. **🧹 Organize** — a local LLM cleans the transcript, extracts tasks, topics, people,
   and goals, and files them into your mind map.
3. **🗺️ Map** — everything you've ever said becomes a living graph of thoughts,
   topics, people, and goals, linked by what they relate to.
4. **📅 Today** — one screen for the day: a sharp "friction" insight, your checklist,
   focus blocks, and what your Apple Calendar already owns.

There is no account, no cloud, and no telemetry. Your voice is transcribed on your
Mac by a local Whisper model; the reasoning runs on a local 1B LLM. Nothing you say
ever leaves this device.

## ✨ What you get

- **One-button capture** with a 20-minute session, live "transcribing → organizing" stages
- **Suggested checklist** — tap to keep the tasks the model heard
- **Mind map** (React Flow) — thoughts orbit the center; topics, people, and goals ring the outside; search your memory in one keystroke
- **Direct voice commands** — "remind me to call Sam" or "start a 25-minute focus" are recognized and made actionable
- **Today view** — friction insight, focus timer with a progress ring, checklist, and calendar sync (read-only, via AppleScript)
- **Auto-update** — checks GitHub Releases and relaunches into the new build when you approve

## 🧱 How it's built

| Layer | Tech |
| --- | --- |
| Shell | [Tauri 2](https://tauri.app) (Rust) + React 19 + Tailwind v4 |
| Speech→Text | `whisper-cli` (whisper.cpp v1.9.5, Metal, optional CoreML encoder) — bundled sidecar |
| Reasoning | `llama-server` (llama.cpp b11609, Metal) hosting Llama 3.2 1B Instruct — bundled sidecar |
| Memory | Local vector store (EmbeddingGemma 2) + SQLite graph store, both in your app-data folder |
| Calendar | Read-only Apple Calendar via `osascript` (graceful when permission is denied) |
| Distribution | DMG with all models bundled · signed updater via GitHub Releases |

The two model servers run on loopback (127.0.0.1) only, are spawned by the app, and
are killed when the app exits.

## 🚀 Try it

### Run a prebuilt app

Download the latest **DMG** from [Releases](https://github.com/abhishek085/Nodemind/releases),
drag Nodemind into Applications, and launch it. The first launch starts the local
engine (a few seconds) — everything is bundled, so it works with no internet at all.

### Build from source

Requirements: macOS (Apple Silicon), Xcode CLT, Rust, Node 18+, cmake.

```sh
# 1. front-end deps
npm install

# 2. fetch the models into src-tauri/resources/models/  (~1.8 GB, one-time)
./scripts/fetch-models.sh

# 3. build the whisper.cpp + llama.cpp sidecar binaries
./scripts/build-sidecars.sh

# 4. build the app (produces src-tauri/target/release/bundle/dmg/Nodemind_*.dmg)
export TAURI_SIGNING_PRIVATE_KEY="$PWD/src-tauri/nodemind.key"   # generate: npx tauri signer generate
npx tauri build
```

For day-to-day development, `npx tauri dev` hot-reloads the UI and reuses the same
sidecars and models.

> **About the signing key** — `src-tauri/nodemind.key` (the updater's *private* key)
> is git-ignored by design. The public key is embedded in `tauri.conf.json`. If you
> fork and build, generate your own keypair (`npx tauri signer generate -w src-tauri/nodemind.key --ci`)
> and update `plugins.updater.pubkey` to match.

## 🔒 Privacy, stated plainly

- Audio is buffered in-process, transcribed locally, and discarded after each capture.
- The SQLite database (thoughts, tasks, graph, vectors, focus sessions) lives in
  `~/Library/Application Support/com.nokast.nodemind/` — open the folder from Settings.
- The only network calls the app can make are: the one-time model download on first
  launch, and the opt-in update check. No analytics, no remote STT, no remote LLM.

## 🗂️ Repository layout

```
├── src/                    # React UI (views, components, lib)
│   ├── lib/                # api (IPC wrappers) · audio (16 kHz worklet) · types
│   ├── views/              # Capture · Graph · Today · Settings
│   └── components/         # Sidebar · StatusBar · SetupScreen · UpdateBanner
├── src-tauri/src/
│   ├── commands/           # the IPC surface the UI talks to
│   ├── engine/             # sidecar lifecycle · whisper · llm · pipeline · prompts
│   ├── store/              # sqlite db · graph · brute-force vector kNN
│   ├── models/             # manifest (sha256-pinned) · resumable downloader
│   ├── calendar/           # read-only Apple Calendar bridge
│   └── lib.rs              # window, tray, menu, exit wiring
├── scripts/
│   ├── fetch-models.sh     # pinned model downloads (resume + sha256 verify)
│   └── build-sidecars.sh   # whisper.cpp v1.9.5 + llama.cpp b11609 (Metal, static)
└── src-tauri/resources/    # prompts.toml + bundled models
```

## 📄 License

MIT — see [LICENSE](LICENSE).
