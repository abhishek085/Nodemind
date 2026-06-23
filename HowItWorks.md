# How Nodemind Works

Nodemind turns a spoken brain-dump into a clean checklist — and it does the whole thing **inside your browser**. Nothing is uploaded.

## The pipeline

```
🎤 You talk
   │
   ▼
🗣️  Speech → text      Whisper runs locally (Transformers.js · WebGPU)
   │
   ▼
🧠 Rant → checklist    A small LLM rewrites it into clear tasks
                       (Llama 3.2 1B via WebLLM · WebGPU)
   │
   ▼
✅ Editable checklist  Saved in your browser (localStorage)
```

## Step by step

1. **Capture** — you tap the mic; the browser records audio until you tap again.
2. **Transcribe** — the audio is converted to 16 kHz and run through Whisper, locally. You get raw text.
3. **Clean up** — the transcript goes to a small local language model with one job: keep only the tasks the person actually said, drop the filler and repetition, merge duplicates, and phrase each as a clear action. It returns a JSON list of tasks.
4. **Review** — tasks render as a checklist you can edit, check off, copy, or delete. Everything is saved locally and survives a refresh.

## Why it stays private

- The speech model and the language model both run on your own device via **WebGPU**.
- There is no backend and no API. The app is just static files your browser executes.
- The only network use is the **one-time model download** on first setup. After that it works offline.

## Design notes

- **Empty input is respected** — if you just muse with no real tasks, you get an empty list, never invented ones.
- **Append mode** — talk again and new tasks merge into your existing list (deduplicated).
- **Model choice** — the 1B model is the sweet spot; smaller models drop tasks, larger ones are slower. It's switchable under *Advanced*.

For the deeper architecture, see [Technical_design.md](Technical_design.md).
