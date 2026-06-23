<p align="center">
  <img src="public/nodemind_icon.png" alt="Nodemind icon" width="110" />
</p>

<h1 align="center">Nodemind</h1>

<p align="center">
  <em>Talk out your messy thoughts. Get a clean checklist.</em>
</p>

<p align="center">
  A dead-simple <strong>voice&#8209;to&#8209;checklist</strong> tool that runs <strong>100% in your browser</strong>.<br />
  Free, private, offline — your voice never leaves your device.
</p>

<p align="center">
  <a href="https://abhishek085.github.io/Nodemind/"><img alt="Open Nodemind" src="https://img.shields.io/badge/▶_Open_Nodemind-6366f1?style=for-the-badge&logoColor=white" /></a>
</p>

<p align="center">
  <img alt="Price" src="https://img.shields.io/badge/price-free-34d399?style=flat-square" />
  <img alt="Privacy" src="https://img.shields.io/badge/runs-100%25_on_device-6366f1?style=flat-square" />
  <img alt="Install" src="https://img.shields.io/badge/install-none-818cf8?style=flat-square" />
  <img alt="PWA" src="https://img.shields.io/badge/PWA-installable-fbbf24?style=flat-square" />
  <img alt="License" src="https://img.shields.io/badge/license-MIT-111111?style=flat-square" />
</p>

<p align="center">
  <sub>a <strong>Nokast</strong> project</sub>
</p>

---

## 💡 The idea

You have too many ideas and you keep losing them. Nodemind solves that one thing:

> ### 🎤 Tap the mic → ramble → get a clean to-do list.

It listens, transcribes your speech, strips the "umm"s and repetition, and turns your rambling into clear, checkable tasks. The twist most tools can't match: **everything runs inside your browser, on your own machine.** Your voice and notes never touch a server.

---

## 🚀 Try it

### **→ [abhishek085.github.io/Nodemind](https://abhishek085.github.io/Nodemind/)**

| | |
|--:|:--|
| **1.** | Open the link in **Chrome** or **Edge** on a laptop/desktop |
| **2.** | Click **Set up Nodemind** once — downloads the AI (~1&nbsp;GB, cached after) |
| **3.** | Tap 🎤 → talk → tap to stop |
| **4.** | Edit ✏️ / check ✅ / copy ⧉ / delete ✕ your tasks — it saves automatically |

> 💡 In Chrome/Edge, click the **install** icon in the address bar to add Nodemind to your dock as a real app. After first setup it works fully **offline**.

---

## ⚖️ How it compares

The problem is popular — but every tool makes you give something up. Most are paid, send your voice to the cloud, or only transcribe without organizing. Nodemind is the one corner nobody else fills.

| | **🧠 Nodemind** | Wispr Flow | AudioPen | Voicenotes | OpenWhispr |
|---|:---:|:---:|:---:|:---:|:---:|
| Voice → organized **checklist** | ✅ | ❌ *(dictation only)* | ✅ | ✅ | ❌ *(transcript only)* |
| **Free** | ✅ | ❌ $15/mo | ❌ $99/yr | ❌ paid | ✅ |
| Runs **on your device** (private) | ✅ | ❌ cloud | ❌ cloud | ❌ cloud | ✅ |
| **No install** (just a link) | ✅ | ❌ app | ✅ | ❌ app | ❌ install |
| **Open source** | ✅ | ❌ | ❌ | ❌ | ✅ |

**Nodemind is the only one that ticks every box.**

### Why it's better

- 🆓 **Free, forever** — no subscriptions, no word-count caps, no "you've hit your limit."
- 🔒 **Genuinely private** — the AI runs in *your* browser. Cloud tools upload your voice; Nodemind never does.
- ✅ **Actually organizes** — dictation tools (Wispr Flow) just type what you say; local tools (OpenWhispr) just transcribe. Nodemind turns the mess into a real list.
- ⚡ **Zero friction** — it's a link, not a download. Nothing to install, no account to create.
- 🔓 **Open source** — inspect it, fork it, trust it.

---

## ✅ Requirements

Running the AI locally is what keeps it free and private — the trade-off is it needs a capable browser and machine.

| | |
|---|---|
| **Browser** | Chrome or Edge (recent). Safari's WebGPU support is still partial. |
| **Device** | Laptop/desktop with ~8&nbsp;GB+ RAM. Phones struggle with the local model. |
| **First load** | One-time ~1&nbsp;GB model download, cached afterward. |
| **Internet** | Needed only for that first setup — works offline after. |

---

## 🛠️ How it works

A single self-contained HTML page with a fully on-device pipeline — no backend, no server:

```
🎤 your voice
   │
   ▼
🗣️  Speech → text      Whisper (transformers.js · WebGPU)
   │
   ▼
🧠 Rant → checklist    Llama 3.2 1B (WebLLM · WebGPU)
   │
   ▼
✅ editable checklist  saved in your browser (localStorage)
```

Every step runs in your browser. The "app" is just static files your device executes.

---

## 🔐 Privacy

- No cloud speech-to-text.
- No hosted LLM — the model runs in your browser.
- No account, no analytics, no telemetry.
- Your audio, transcripts, and tasks stay on your device.

---

## 💻 Run it locally

No build step — it's a static page. The mic and WebGPU need a secure context, which `localhost` provides, so just serve the folder:

```bash
git clone https://github.com/abhishek085/Nodemind.git
cd Nodemind
python3 -m http.server 8000
```

Then open **<http://localhost:8000/poc/voice-checklist.html>** in Chrome or Edge. *(Any static server works — e.g. `npx serve`.)*

---

## 🧩 Tech stack

- **Speech-to-text:** `whisper-base` via [Transformers.js](https://github.com/huggingface/transformers.js) (WebGPU)
- **Task cleanup:** [Llama 3.2 1B](https://huggingface.co/meta-llama/Llama-3.2-1B) via [WebLLM](https://github.com/mlc-ai/web-llm) (WebGPU)
- **App:** one static HTML/CSS/JS file — no framework, no build step
- **Offline / installable:** Web App Manifest + service worker (PWA)
- **Hosting:** GitHub Pages (served from the `gh-pages` branch)

---

## 📁 Project structure

- [`poc/voice-checklist.html`](poc/voice-checklist.html) — the entire app (UI, voice capture, STT, LLM cleanup, checklist)
- [`poc/manifest.webmanifest`](poc/manifest.webmanifest) · [`poc/sw.js`](poc/sw.js) — PWA manifest + offline service worker
- [`HowItWorks.md`](HowItWorks.md) — the pipeline in plain language
- [`Technical_design.md`](Technical_design.md) — architecture & design principles

> **History:** Nodemind started as a native macOS app (Tauri + Rust + Ollama) before pivoting to this zero-install web app. The original desktop source lives in the project's git history.

---

## 🗺️ Roadmap

- [ ] Smart grouping (Today / This week / Someday)
- [ ] One-tap export to Apple Reminders / Todoist / Notion
- [ ] Edit-by-voice ("remove the dentist one")
- [ ] Wider browser support as WebGPU matures

---

## 🤝 Contributing

Contributions are welcome. Keep PRs small and focused, and preserve the on-device, no-cloud design.

---

## 🌱 About Nokast

Nodemind is built by **Nokast** — tools for messy, fast-moving minds. The goal is simple software that meets you where your thinking actually is: spoken, scattered, and in motion.

## 📄 License

See [LICENSE](LICENSE). MIT unless noted otherwise for third-party components.
