// The flagship view: one button → brain-dump → a clean, structured result.
//
// States: idle → recording → processing (transcribing/organizing) → result.
// The backend pipeline owns the heavy lifting; this view only streams audio
// in and renders whatever comes back.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import * as api from "../lib/api";
import { Recorder } from "../lib/audio";
import type { CaptureResult, CaptureStage, Settings } from "../lib/types";

interface Props {
  settings: Settings | null;
  onCaptured: () => void; // lets App refresh dependent views
}

type Phase =
  | { name: "idle" }
  | { name: "recording" }
  | { name: "processing"; stage: CaptureStage }
  | { name: "result"; result: CaptureResult };

export default function CaptureView({ settings, onCaptured }: Props) {
  const [phase, setPhase] = useState<Phase>({ name: "idle" });
  const [seconds, setSeconds] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [lang, setLang] = useState(settings?.language ?? "en");
  const recorderRef = useRef<Recorder | null>(null);
  const timerRef = useRef<number | null>(null);

  // Keep the language in sync with settings without resetting an in-flight phase.
  useEffect(() => {
    if (settings) setLang(settings.language);
  }, [settings]);

  useEffect(
    () => () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
      recorderRef.current?.stop();
    },
    [],
  );

  const finish = useCallback(
    async () => {
      setPhase({ name: "processing", stage: "transcribing" });
      try {
        const result = await api.finishCapture();
        setPhase({ name: "result", result });
        onCaptured();
      } catch (e) {
        setError(`Capture failed: ${String(e)}`);
        setPhase({ name: "idle" });
      }
    },
    [onCaptured],
  );

  const start = async () => {
    setError(null);
    try {
      const session = await api.beginCapture(lang);
      setSeconds(0);
      const rec = new Recorder(session.maxSeconds, {
        onChunk: (b64) => {
          api.appendAudio(b64).catch(() => undefined); // buffer grows in Rust
        },
        onEnded: () => {
          if (timerRef.current !== null) {
            window.clearInterval(timerRef.current);
            timerRef.current = null;
          }
          void finish();
        },
        onError: (message) => {
          setError(message);
          setPhase({ name: "idle" });
        },
      });
      recorderRef.current = rec;
      await rec.start();
      if (!rec.running) return; // error already surfaced via onError
      setPhase({ name: "recording" });
      timerRef.current = window.setInterval(() => setSeconds((s) => s + 1), 1000);
    } catch (e) {
      setError(String(e));
    }
  };

  const stop = () => recorderRef.current?.stop();

  // Track stage events while a capture is being processed.
  useEffect(() => {
    const un = api.on<{ stage: CaptureStage }>("capture-stage", (e) => {
      setPhase((p) =>
        p.name === "processing" && e.stage !== "done"
          ? { name: "processing", stage: e.stage }
          : p,
      );
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto p-8">
      <div className="w-full max-w-2xl">
        {phase.name === "idle" && <IdleCard onStart={start} lang={lang} onLang={setLang} />}
        {phase.name === "recording" && <RecordingCard seconds={seconds} onStop={stop} />}
        {phase.name === "processing" && <ProcessingCard stage={phase.stage} />}
        {phase.name === "result" && (
          <ResultCard
            result={phase.result}
            onDone={() => setPhase({ name: "idle" })}
            onCaptured={onCaptured}
          />
        )}
        {error && (
          <div className="mt-4 rounded-xl border border-danger/30 bg-danger/10 px-4 py-3 text-xs text-danger">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}

// ── states ──────────────────────────────────────────────────────────────────

function IdleCard({
  onStart,
  lang,
  onLang,
}: {
  onStart: () => void;
  lang: string;
  onLang: (l: string) => void;
}) {
  return (
    <div className="flex flex-col items-center rounded-2xl bg-surface p-10 shadow-sm backdrop-blur-xl fade-up">
      <h2 className="mb-1 text-xl font-bold">What's on your mind?</h2>
      <p className="mb-8 max-w-sm text-center text-sm text-ink-2">
        Talk — messily, in English or Hindi. Nodemind listens, cleans it up,
        and files the tasks, topics, and people into your mind map.
      </p>
      <button
        onClick={onStart}
        className="flex h-28 w-28 items-center justify-center rounded-full bg-gradient-to-br from-accent to-accent-2 text-white shadow-md transition-transform hover:scale-105 active:scale-95"
        title="Start recording"
      >
        <svg viewBox="0 0 24 24" className="h-10 w-10" fill="none" stroke="currentColor" strokeWidth="1.8">
          <rect x="9" y="3" width="6" height="11" rx="3" />
          <path d="M5 11a7 7 0 0 0 14 0M12 18v3" strokeLinecap="round" />
        </svg>
      </button>
      <p className="mt-4 text-xs text-ink-3">Tap to start · 20-minute sessions</p>
      <div className="mt-6 flex gap-1 rounded-full bg-surface-2 p-1">
        {(["en", "hi"] as const).map((l) => (
          <button
            key={l}
            onClick={() => onLang(l)}
            className={`rounded-full px-3 py-1 text-xs font-semibold transition-colors ${
              lang === l ? "bg-surface text-ink shadow-sm" : "text-ink-3 hover:text-ink-2"
            }`}
          >
            {l === "en" ? "English" : "Hinglish"}
          </button>
        ))}
      </div>
    </div>
  );
}

function fmtClock(s: number): string {
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`;
}

function RecordingCard({ seconds, onStop }: { seconds: number; onStop: () => void }) {
  return (
    <div className="flex flex-col items-center rounded-2xl bg-surface p-10 shadow-sm backdrop-blur-xl fade-up">
      <p className="mb-6 text-sm font-semibold text-ink-2">Listening…</p>
      <button
        onClick={onStop}
        className="rec-pulse flex h-28 w-28 items-center justify-center rounded-full bg-danger text-white shadow-md transition-transform hover:scale-105 active:scale-95"
        title="Stop"
      >
        <span className="h-9 w-9 rounded-md bg-white" />
      </button>
      <p className="mt-5 font-mono text-2xl tabular-nums text-ink">{fmtClock(seconds)}</p>
      <p className="mt-2 text-xs text-ink-3">Tap the square to stop and process</p>
    </div>
  );
}

const STAGE_LABEL: Record<CaptureStage, string> = {
  transcribing: "Transcribing your words…",
  organizing: "Organizing into checklist and mind map…",
  done: "Done",
};

function ProcessingCard({ stage }: { stage: CaptureStage }) {
  return (
    <div className="flex flex-col items-center rounded-2xl bg-surface p-12 shadow-sm backdrop-blur-xl fade-up">
      <div className="h-10 w-10 animate-spin rounded-full border-2 border-line border-t-accent" />
      <p className="mt-5 text-sm text-ink-2">{STAGE_LABEL[stage]}</p>
      <p className="mt-1 text-xs text-ink-3">
        Runs entirely on this Mac — a minute of audio takes a few seconds.
      </p>
    </div>
  );
}

// ── result ──────────────────────────────────────────────────────────────────

function Chip({ children, tone }: { children: ReactNode; tone: "accent" | "ok" | "neutral" }) {
  const cls =
    tone === "accent"
      ? "bg-accent/12 text-accent"
      : tone === "ok"
        ? "bg-ok/12 text-ok"
        : "bg-surface-2 text-ink-2";
  return (
    <span className={`rounded-full px-2.5 py-1 text-[11px] font-medium ${cls}`}>{children}</span>
  );
}

function ResultCard({
  result,
  onDone,
  onCaptured,
}: {
  result: CaptureResult;
  onDone: () => void;
  onCaptured: () => void;
}) {
  const [added, setAdded] = useState<Set<number>>(new Set());

  const addTask = async (title: string, i: number) => {
    try {
      await api.addTask(title);
      setAdded((prev) => new Set(prev).add(i));
      onCaptured();
    } catch {
      // task card just stays un-added
    }
  };

  const startParsedFocus = async () => {
    const c = result.command;
    if (!c) return;
    try {
      if (c.intent === "start_focus") {
        await api.startFocus(c.minutes ?? undefined);
        onDone(); // the Today view owns the countdown
      }
    } catch (e) {
      alert(String(e));
    }
  };

  return (
    <div className="space-y-4 fade-up">
      <div className="rounded-2xl bg-surface p-6 shadow-sm backdrop-blur-xl">
        <div className="mb-2 flex items-baseline justify-between">
          <h3 className="text-sm font-bold">Your brain-dump</h3>
          <span className="text-[11px] text-ink-3">
            {Math.max(1, Math.round(result.durationSecs / 60) || 1)} min ·{" "}
            {result.transcript.split(/\s+/).length} words
          </span>
        </div>
        <p className="max-h-40 overflow-y-auto whitespace-pre-wrap text-[13px] leading-relaxed text-ink-2">
          {result.transcript}
        </p>
        {result.summary && (
          <p className="mt-3 rounded-lg bg-accent/8 px-3 py-2 text-xs text-ink">
            <span className="font-semibold text-accent">In short: </span>
            {result.summary}
          </p>
        )}
      </div>

      {result.tasks.length > 0 && (
        <div className="rounded-2xl bg-surface p-6 shadow-sm backdrop-blur-xl">
          <h3 className="mb-3 text-sm font-bold">
            Suggested checklist <span className="font-normal text-ink-3">— tap to keep</span>
          </h3>
          <ul className="space-y-2">
            {result.tasks.map((t, i) => (
              <li key={i} className="flex items-center gap-3">
                <button
                  onClick={() => addTask(t, i)}
                  disabled={added.has(i)}
                  className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                    added.has(i)
                      ? "bg-ok/12 text-ok"
                      : "bg-surface-2 text-ink hover:bg-accent/15 hover:text-accent"
                  }`}
                >
                  {added.has(i) ? "✓ Added to Today" : "+ Keep"}
                </button>
                <span
                  className={`text-[13px] ${
                    added.has(i) ? "text-ink-3 line-through" : "text-ink-2"
                  }`}
                >
                  {t}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {(result.topics.length > 0 || result.people.length > 0 || result.command) && (
        <div className="rounded-2xl bg-surface p-6 shadow-sm backdrop-blur-xl">
          <h3 className="mb-3 text-sm font-bold">Filed into your mind map</h3>
          <div className="flex flex-wrap gap-2">
            {result.topics.map((t) => (
              <Chip key={`t-${t}`} tone="accent">
                #{t}
              </Chip>
            ))}
            {result.people.map((p) => (
              <Chip key={`p-${p}`} tone="neutral">
                👤 {p}
              </Chip>
            ))}
            {result.command &&
              (result.command.intent === "start_focus" ? (
                <button
                  onClick={startParsedFocus}
                  className="rounded-full bg-accent px-3 py-1 text-[11px] font-semibold text-white hover:opacity-90"
                >
                  ▶ Start {result.command.minutes ?? 25}-min focus
                </button>
              ) : (
                <Chip tone="ok">✓ {result.command.title ?? "Task"} added</Chip>
              ))}
          </div>
        </div>
      )}

      <div className="flex justify-end">
        <button
          onClick={onDone}
          className="rounded-xl bg-surface px-5 py-2 text-sm font-semibold text-ink-2 shadow-sm hover:text-ink"
        >
          Done
        </button>
      </div>
    </div>
  );
}
