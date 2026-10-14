// The menu-bar popover: record and review without opening the full app.
//
// Rendered in the `quick` window (see src-tauri/src/tray.rs). The full window
// is one click away for the mind map, Today, and settings.

import { useEffect, useState } from "react";
import * as api from "../lib/api";
import {
  DualMeter,
  LevelMeter,
  LivePartial,
  MicButton,
  NothingHeard,
  Processing,
  RecordingHint,
  ResultBody,
  StopButton,
  fmtClock,
} from "../components/capture";
import { SearchInput, SearchResults, useSearch } from "../components/search";
import { useCapture } from "../lib/useCapture";
import type { EngineStatus } from "../lib/types";

export default function QuickCapture() {
  const cap = useCapture();
  const search = useSearch(5);
  const [status, setStatus] = useState<EngineStatus | null>(null);

  useEffect(() => {
    void api.engineStatus().then(setStatus).catch(() => undefined);
    const un = api.on<EngineStatus>("engine-status", setStatus);
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const ready = status?.phase === "ready";
  const { phase } = cap;

  return (
    <div className="flex h-full flex-col overflow-hidden rounded-xl border border-line bg-bg text-ink">
      <header className="flex items-center justify-between border-b border-line px-4 py-2.5">
        <span className="text-sm font-bold">Nodemind</span>
        <span className="flex items-center gap-1.5 text-[11px] text-ink-3">
          <span className={`h-1.5 w-1.5 rounded-full ${ready ? "bg-ok" : "bg-amber-400"}`} />
          {ready ? "Ready" : status?.phase === "setup" ? "Needs setup" : "Starting…"}
        </span>
      </header>

      {phase.name === "idle" && ready && (
        <div className="px-3 pt-3">
          <SearchInput value={search.query} onChange={search.setQuery} busy={search.busy} placeholder="Search your notes…" />
        </div>
      )}

      <main className="flex min-h-0 flex-1 flex-col items-center justify-center overflow-y-auto px-5 py-4 text-center">
        {phase.name === "idle" && search.query.trim().length >= 2 && (
          <div className="w-full self-start">
            <SearchResults hits={search.hits} query={search.query} error={search.error} compact />
          </div>
        )}

        {phase.name === "idle" && search.query.trim().length < 2 && (
          <>
            <MicButton size="md" onClick={() => (ready ? void cap.start() : void api.openMainWindow())} />
            <p className="mt-3 text-xs text-ink-3">
              {ready ? (
                <>
                  Tap, or hold <kbd className="font-mono">⌥⇧Space</kbd> anywhere
                </>
              ) : (
                "Finish setup in the main window first"
              )}
            </p>
            {ready && (
              <button
                onClick={() => void cap.start(undefined, "meeting")}
                className="mt-4 rounded-full border border-line px-3 py-1 text-xs font-medium text-ink-2 hover:border-accent hover:text-accent"
              >
                Take meeting notes
              </button>
            )}
          </>
        )}

        {phase.name === "recording" && (
          <>
            <StopButton size="md" onClick={() => void cap.stop()} />
            <p className="mt-3 font-mono text-xl tabular-nums">{fmtClock(cap.seconds)}</p>
            {cap.mode === "meeting" ? <DualMeter you={cap.level} them={cap.themLevel} /> : <LevelMeter level={cap.level} />}
            {cap.warning && <p className="mt-2 text-[11px] text-amber-500">{cap.warning}</p>}
            <RecordingHint micDead={cap.micDead} />
            <LivePartial text={cap.partial} className="mt-4 w-full" />
            <button onClick={() => void cap.cancel()} className="mt-3 text-[11px] text-ink-3 hover:text-danger">
              Discard
            </button>
          </>
        )}

        {phase.name === "processing" && <Processing stage={phase.stage} />}

        {phase.name === "result" && phase.result.transcript === "" && <NothingHeard onRetry={cap.reset} />}

        {phase.name === "result" && phase.result.transcript !== "" && (
          <div className="w-full self-start">
            <ResultBody result={phase.result} onChange={cap.setResult} compact />
            <button
              onClick={cap.reset}
              className="mt-4 w-full rounded-lg bg-surface-2 py-1.5 text-xs font-semibold text-ink-2 hover:text-ink"
            >
              New capture
            </button>
          </div>
        )}

        {cap.error && <p className="mt-3 text-[11px] text-danger">{cap.error}</p>}
      </main>

      <footer className="flex gap-2 border-t border-line px-3 py-2">
        <button
          onClick={() => void api.openMainWindow("graph")}
          className="flex-1 rounded-lg px-2 py-1.5 text-xs font-medium text-ink-2 hover:bg-surface-2 hover:text-ink"
        >
          Mind map
        </button>
        <button
          onClick={() => void api.openMainWindow("today")}
          className="flex-1 rounded-lg px-2 py-1.5 text-xs font-medium text-ink-2 hover:bg-surface-2 hover:text-ink"
        >
          Today
        </button>
        <button
          onClick={() => void api.openMainWindow()}
          className="flex-1 rounded-lg px-2 py-1.5 text-xs font-medium text-ink-2 hover:bg-surface-2 hover:text-ink"
        >
          Open app
        </button>
      </footer>
    </div>
  );
}
