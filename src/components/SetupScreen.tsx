// First-launch screen: one-time model download, then engine boot.
// Shown whenever the backend reports the `setup` phase.

import { useEffect, useState } from "react";
import { beginSetup } from "../lib/api";
import type { EngineStatus } from "../lib/types";

interface Props {
  status: EngineStatus;
  onReady: () => void;
}

function fmtGB(bytes: number): string {
  return `${(bytes / (1024 ** 3)).toFixed(2)} GB`;
}

export default function SetupScreen({ status, onReady }: Props) {
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const anyMissing = status.models.some((m) => !m.present);
  const anyDownloading = status.models.some((m) => m.downloading);
  const anyErrored = status.models.some((m) => m.error !== null);

  useEffect(() => {
    if (status.phase === "ready") onReady();
  }, [status.phase, onReady]);

  const totalMissing = status.models
    .filter((m) => !m.present)
    .reduce((sum, m) => sum + m.expectedBytes, 0);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      await beginSetup();
    } catch (e) {
      setError(String(e));
      setStarting(false);
    }
  };

  return (
    <div className="flex flex-1 items-center justify-center p-8">
      <div className="w-full max-w-lg rounded-2xl bg-surface p-8 shadow-md backdrop-blur-xl fade-up">
        <div className="mb-1 flex items-center gap-2">
          <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-gradient-to-br from-accent to-accent-2 text-white">
            <svg viewBox="0 0 24 24" className="h-4.5 w-4.5" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="7" cy="7" r="2.5" />
              <circle cx="17" cy="9" r="2.5" />
              <circle cx="11" cy="17" r="2.5" />
              <path d="M8.8 8.3 14.7 9M8 9.2l2.2 5.4M15.5 11l-3 4.4" strokeWidth="1.5" />
            </svg>
          </span>
          <h1 className="text-lg font-bold">Welcome to Nodemind</h1>
        </div>
        <p className="mb-5 text-sm leading-relaxed text-ink-2">
          Your personal thinking assistant — 100% local. Before the first brain-dump,
          Nodemind needs three small models (about {fmtGB(totalMissing)}). They download
          once, then everything — transcription, reasoning, memory — runs on this Mac.
          Nothing ever leaves the device.
        </p>

        <div className="mb-5 space-y-3">
          {status.models.map((m) => {
            const pct =
              m.expectedBytes > 0 ? Math.min(100, (m.downloadedBytes / m.expectedBytes) * 100) : 0;
            return (
              <div key={m.key} className="rounded-xl border border-line bg-surface-2 p-3">
                <div className="mb-1.5 flex items-baseline justify-between">
                  <span className="text-xs font-semibold">{m.label}</span>
                  <span className="text-[11px] text-ink-3">
                    {m.error
                      ? m.error
                      : m.present
                        ? "ready"
                        : m.downloading
                          ? `${pct.toFixed(0)}% · ${fmtGB(m.downloadedBytes)} of ${fmtGB(m.expectedBytes)}`
                          : fmtGB(m.expectedBytes)}
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-line">
                  <div
                    className={`h-full rounded-full transition-all duration-300 ${
                      m.error ? "bg-danger" : m.present ? "bg-ok" : "bg-accent"
                    }`}
                    style={{ width: m.present ? "100%" : `${pct}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>

        {error && <p className="mb-3 text-xs text-danger">{error}</p>}

        <button
          onClick={start}
          disabled={starting || anyDownloading || (anyErrored && !anyMissing)}
          className="w-full rounded-xl bg-accent py-2.5 text-sm font-semibold text-white transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {anyDownloading
            ? "Downloading models…"
            : anyMissing
              ? "Download & set up"
              : anyErrored
                ? "Try again"
                : "Get started"}
        </button>
        <p className="mt-3 text-center text-[11px] text-ink-3">
          The engine starts automatically once every model is in place.
        </p>
      </div>
    </div>
  );
}
