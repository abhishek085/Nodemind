// The flagship view: one button → brain-dump → a clean, structured result.
//
// States: idle → recording (live transcript) → processing → result.
// The backend owns the mic and the pipeline; this view renders the shared
// capture state, so a capture started from the menu bar shows up here too.

import { useEffect, useState } from "react";
import {
  DualMeter,
  LevelMeter,
  LivePartial,
  MicButton,
  ModeSwitch,
  NothingHeard,
  Processing,
  RecordingHint,
  ResultBody,
  StopButton,
  fmtClock,
} from "../components/capture";
import { useCapture } from "../lib/useCapture";
import type { CaptureMode, Settings } from "../lib/types";

interface Props {
  settings: Settings | null;
  onCaptured: () => void; // lets App refresh dependent views
}

export default function CaptureView({ settings, onCaptured }: Props) {
  const cap = useCapture();
  const [lang, setLang] = useState(settings?.language ?? "en");
  const [mode, setMode] = useState<CaptureMode>("note");

  // Keep the language in sync with settings without resetting an in-flight phase.
  useEffect(() => {
    if (settings) setLang(settings.language);
  }, [settings]);

  useEffect(() => {
    if (cap.phase.name === "result") onCaptured();
  }, [cap.phase.name, onCaptured]);

  const { phase } = cap;
  return (
    <div className="flex flex-1 flex-col overflow-y-auto p-8">
      {/* my-auto: centred when short, scrolls from the top when tall. */}
      <div className="mx-auto my-auto w-full max-w-2xl">
        <div className="flex flex-col items-center rounded-2xl bg-surface p-10 text-center shadow-sm backdrop-blur-xl fade-up">
          {phase.name === "idle" && (
            <>
              <div className="mb-6">
                <ModeSwitch mode={mode} onMode={setMode} />
              </div>
              <h2 className="mb-1 text-xl font-bold">
                {mode === "note" ? "What's on your mind?" : "Take notes on a call"}
              </h2>
              <p className="mb-8 max-w-sm text-sm text-ink-2">
                {mode === "note"
                  ? "Talk — messily, in English or Hinglish. Nodemind listens, cleans it up, and files the tasks, topics, and people into your mind map."
                  : "Nodemind records you and the call audio on this Mac, labels who said what, and writes the minutes. Your action items go onto Today."}
              </p>
              <MicButton onClick={() => void cap.start(lang, mode)} />
              <p className="mt-4 text-xs text-ink-3">
                {mode === "note" ? (
                  <>
                    Tap to start · or hold <kbd className="font-mono">⌥⇧Space</kbd> anywhere
                  </>
                ) : (
                  "Works with Zoom, Meet, Teams — anything playing on this Mac. Use headphones for the cleanest notes."
                )}
              </p>
              <div className="mt-6 flex gap-1 rounded-full bg-surface-2 p-1">
                {(["en", "hi"] as const).map((l) => (
                  <button
                    key={l}
                    onClick={() => setLang(l)}
                    className={`rounded-full px-3 py-1 text-xs font-semibold transition-colors ${
                      lang === l ? "bg-surface text-ink shadow-sm" : "text-ink-3 hover:text-ink-2"
                    }`}
                  >
                    {l === "en" ? "English" : "Hinglish"}
                  </button>
                ))}
              </div>
            </>
          )}

          {phase.name === "recording" && (
            <>
              <p className="mb-6 text-sm font-semibold text-ink-2">
                {cap.mode === "meeting" ? "Taking meeting notes…" : "Listening…"}
              </p>
              <StopButton onClick={() => void cap.stop()} />
              <p className="mt-5 font-mono text-2xl tabular-nums text-ink">{fmtClock(cap.seconds)}</p>
              {cap.mode === "meeting" ? <DualMeter you={cap.level} them={cap.themLevel} /> : <LevelMeter level={cap.level} />}
              {cap.warning && <p className="mt-2 text-xs text-amber-500">{cap.warning}</p>}
              <RecordingHint micDead={cap.micDead} />
              <LivePartial text={cap.partial} className="mt-6 w-full" />
            </>
          )}

          {phase.name === "processing" && (
            <>
              <Processing stage={phase.stage} />
              <LivePartial text={cap.partial} className="mt-6 w-full opacity-70" />
            </>
          )}

          {phase.name === "result" && phase.result.transcript === "" && (
            <NothingHeard onRetry={cap.reset} />
          )}

          {phase.name === "result" && phase.result.transcript !== "" && (
            <div className="w-full">
              <ResultBody result={phase.result} onChange={cap.setResult} />
              <div className="mt-6 flex justify-end">
                <button
                  onClick={cap.reset}
                  className="rounded-xl bg-surface-2 px-5 py-2 text-sm font-semibold text-ink-2 hover:text-ink"
                >
                  Done
                </button>
              </div>
            </div>
          )}
        </div>

        {cap.error && (
          <div className="mt-4 rounded-xl border border-danger/30 bg-danger/10 px-4 py-3 text-xs text-danger">
            {cap.error}
          </div>
        )}
      </div>
    </div>
  );
}
