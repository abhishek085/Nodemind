<p align="center">
  <img src="public/nodemind_icon.png" alt="Nodemind icon" width="120" />
</p>

<h1 align="center">Nodemind</h1>

<p align="center">
  Talk out your messy thoughts. Get a clean checklist.<br />
  A dead-simple voice-to-checklist tool that runs <strong>100% in your browser</strong> — free, private, and offline.
</p>

<p align="center">
  <a href="https://abhishek085.github.io/Nodemind/">
    <img alt="Open Nodemind" src="https://img.shields.io/badge/Open-Nodemind-6366f1?style=for-the-badge&logo=googlechrome&logoColor=white" />
  </a>
</p>

<p align="center">
  <strong>No install</strong> • <strong>No account</strong> • <strong>No cloud</strong> • <strong>No API key</strong> • <strong>Open source</strong>
</p>

## What it is

You have too many ideas and you keep losing them. Nodemind fixes exactly that one thing:

> **Tap the mic → ramble for 30 seconds → get a clean, organized to-do list.**

It listens, transcribes your speech, strips the "umm"s and repetition, and turns your rambling into clear, checkable tasks. Then you can edit, check off, copy, or delete them.

The catch most tools have — *"your voice goes to our servers"* — doesn't apply here. **Everything runs inside your browser, on your own device.** Your voice and your notes never leave your computer.

## Try it

👉 **[abhishek085.github.io/Nodemind](https://abhishek085.github.io/Nodemind/)**

1. Open the link in **Chrome or Edge** on a laptop/desktop.
2. Click **Set up Nodemind** once — this downloads the AI (~1 GB). It's cached afterward, so it only happens the first time.
3. Tap the 🎤 button, talk, then tap again to stop.
4. Your checklist appears. Edit / check / copy / delete as you like — it saves automatically.

**Tip:** In Chrome/Edge, click the **install** icon in the address bar to add Nodemind to your dock as a standalone app. After the first setup it even works with no internet.

## Requirements

Running the AI locally is what keeps it private and free — the tradeoff is that it needs a capable browser and machine:

| | |
|---|---|
| **Browser** | Chrome or Edge (recent). Safari's WebGPU support is still partial. |
| **Device** | A laptop/desktop with ~8 GB+ RAM. Phones struggle with the local model. |
| **First load** | A one-time ~1 GB model download. Cached afterward. |
| **Internet** | Needed only for that first setup. Works offline after. |

## How it works

A single self-contained HTML page with a fully on-device pipeline:

1. **Capture** — the browser records your microphone (you tap to start/stop).
2. **Speech → text** — [Whisper](https://github.com/openai/whisper) runs locally in the browser via [Transformers.js](https://github.com/huggingface/transformers.js) on WebGPU.
3. **Rant → checklist** — a small local LLM ([Llama 3.2 1B](https://huggingface.co/meta-llama/Llama-3.2-1B) via [WebLLM](https://github.com/mlc-ai/web-llm), also WebGPU) rewrites the transcript into clean, deduplicated, imperative tasks.
4. **Review** — tasks render as an editable checklist, saved to `localStorage`.

No backend. No server-side anything. The "app" is just static files your browser runs.

## Privacy

- No cloud speech-to-text.
- No hosted LLM — the model runs in your browser.
- No account, no analytics, no telemetry.
- Your audio, transcripts, and tasks stay on your device.

## Tech stack

- **Speech-to-text:** `whisper-base` via Transformers.js (WebGPU)
- **Task cleanup:** Llama 3.2 1B via WebLLM (WebGPU)
- **App:** a single static HTML/CSS/JS file — no framework, no build step
- **Offline / installable:** Web App Manifest + a service worker (PWA)
- **Hosting:** GitHub Pages (served from the `gh-pages` branch)

## Run it locally

No build step — it's a static page. The microphone and WebGPU require a secure context, which `localhost` satisfies, so just serve the folder:

```bash
git clone https://github.com/abhishek085/Nodemind.git
cd Nodemind
python3 -m http.server 8000
```

Then open <http://localhost:8000/poc/voice-checklist.html> in Chrome or Edge.

(Any static file server works — e.g. `npx serve`.)

## Project structure

- [`poc/voice-checklist.html`](poc/voice-checklist.html) — the entire app (UI, voice capture, STT, LLM cleanup, checklist)
- [`poc/manifest.webmanifest`](poc/manifest.webmanifest), [`poc/sw.js`](poc/sw.js) — PWA manifest + offline service worker
- [`poc/icon-192.png`](poc/icon-192.png), [`poc/icon-512.png`](poc/icon-512.png) — app icons

> **Note:** Nodemind began as a native macOS app (Tauri + Rust + Whisper + Ollama). It has since pivoted to this simpler, zero-install web app. The original desktop source still lives under [`src/`](src) and [`src-tauri/`](src-tauri) for reference, but the web app in [`poc/`](poc) is the active product.

## Roadmap

- [ ] Smarter grouping (Today / This week / Someday)
- [ ] One-tap export to Apple Reminders / Todoist / Notion
- [ ] Edit-by-voice ("remove the dentist one")
- [ ] Wider browser support as WebGPU matures

## Contributing

Contributions are welcome. Keep PRs small and focused, and preserve the on-device, no-cloud design.

## License

See [LICENSE](LICENSE). MIT unless noted otherwise for third-party components.
