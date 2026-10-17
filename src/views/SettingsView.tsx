// Settings: engine health, model status, language, data, updates.

import { useCallback, useEffect, useState } from "react";
import * as api from "../lib/api";
import type { EngineStatus, Settings } from "../lib/types";

interface Props {
  settings: Settings | null;
  status: EngineStatus | null;
  onSettings: (s: Settings) => void;
}

function fmtSize(bytes: number): string {
  if (bytes <= 0) return "—";
  if (bytes > 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${(bytes / 1024 ** 2).toFixed(0)} MB`;
}

export default function SettingsView({ settings, status, onSettings }: Props) {
  const [busy, setBusy] = useState<string | null>(null);
  const [reindexed, setReindexed] = useState<number | null>(null);
  const [checking, setChecking] = useState(false);
  const [statusLocal, setStatusLocal] = useState<EngineStatus | null>(status);

  const onStatus = useCallback((s: EngineStatus) => {
    setStatusLocal(s);
  }, []);

  const refresh = useCallback(() => {
    void api
      .getSettings()
      .then(onSettings)
      .catch(() => undefined);
    void api
      .engineStatus()
      .then(onStatus)
      .catch(() => undefined);
  }, [onSettings, onStatus]);

  // Track live engine status while this view is open.
  useEffect(() => {
    const un = api.on<EngineStatus>("engine-status", onStatus);
    return () => {
      void un.then((f) => f());
    };
  }, [onStatus]);

  const reindex = async () => {
    setBusy("reindex");
    setReindexed(null);
    try {
      setReindexed(await api.reindexEmbeddings());
    } catch (e) {
      alert(`Reindex failed: ${String(e)}`);
    } finally {
      setBusy(null);
      refresh();
    }
  };

  const checkUpdates = async () => {
    setChecking(true);
    try {
      const u = await api.checkForUpdates();
      alert(
        u
          ? `Version ${u.version} is available — use the banner to install.`
          : "You're on the latest version.",
      );
    } catch (e) {
      alert(
        `Update check failed: ${String(e)}\n\n(Updates need an internet connection; the app itself works offline.)`,
      );
    } finally {
      setChecking(false);
    }
  };

  const s = statusLocal ?? status;

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mx-auto max-w-2xl space-y-4">
        {/* engine */}
        <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-bold">Local engine</h3>
            <div className="flex items-center gap-2">
              <span
                className={`rounded-full px-2 py-0.5 text-[10px] font-semibold ${
                  s?.phase === "ready"
                    ? "bg-ok/15 text-ok"
                    : s?.phase === "failed"
                      ? "bg-danger/15 text-danger"
                      : "bg-warn/15 text-warn"
                }`}
              >
                {s?.phase === "ready"
                  ? "READY"
                  : s?.phase === "failed"
                    ? "ERROR"
                    : s?.phase === "starting"
                      ? "STARTING"
                      : "SETUP"}
              </span>
            </div>
          </div>

          {s?.error && (
            <p className="mb-3 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger">{s.error}</p>
          )}

          <div className="space-y-2.5">
            {s?.models.map((m) => (
              <div key={m.key} className="rounded-xl border border-line bg-surface-2 p-3">
                <div className="mb-1 flex items-baseline justify-between">
                  <span className="text-xs font-semibold">{m.label}</span>
                  <span className="text-[11px] text-ink-3">
                    {m.error
                      ? m.error
                      : m.present
                        ? fmtSize(m.sizeBytes)
                        : m.downloading
                          ? `${((m.downloadedBytes / Math.max(1, m.expectedBytes)) * 100).toFixed(0)}%`
                          : "missing"}
                  </span>
                </div>
                {m.downloading && (
                  <div className="h-1 overflow-hidden rounded-full bg-line">
                    <div
                      className="h-full rounded-full bg-accent transition-all"
                      style={{
                        width: `${(m.downloadedBytes / Math.max(1, m.expectedBytes)) * 100}%`,
                      }}
                    />
                  </div>
                )}
              </div>
            ))}
          </div>

          <div className="mt-3 flex items-center gap-2">
            <button
              onClick={refresh}
              className="rounded-lg border border-line px-3 py-1.5 text-xs text-ink-2 hover:text-ink"
            >
              Refresh status
            </button>
            <button
              onClick={reindex}
              disabled={busy !== null}
              className="rounded-lg border border-line px-3 py-1.5 text-xs text-ink-2 hover:text-ink disabled:opacity-50"
            >
              {busy === "reindex" ? "Reindexing…" : "Re-embed all notes"}
            </button>
            {reindexed !== null && (
              <span className="text-[11px] text-ink-3">{reindexed} vectors</span>
            )}
          </div>
          <p className="mt-2 text-[11px] text-ink-3">
            Architecture: {settings?.arch ?? s?.arch ?? "…"} · v{settings?.version ?? "…"}
          </p>
        </section>

        {/* language */}
        <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
          <h3 className="mb-2 text-sm font-bold">Language</h3>
          <p className="mb-3 text-xs text-ink-2">
            What you usually speak in. “Hinglish” decodes as English and translates Hindi
            words, so the mind map stays in English either way.
          </p>
          <div className="flex gap-2">
            {(["en", "hi"] as const).map((l) => (
              <button
                key={l}
                disabled={settings?.language === l}
                onClick={() => void api.setLanguage(l).then(refresh)}
                className={`rounded-xl px-4 py-2 text-xs font-semibold transition-colors ${
                  settings?.language === l
                    ? "bg-accent text-white"
                    : "border border-line text-ink-2 hover:text-ink"
                }`}
              >
                {l === "en" ? "English" : "Hinglish"}
              </button>
            ))}
          </div>
        </section>

        {/* data */}
        <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
          <h3 className="mb-2 text-sm font-bold">Your data</h3>
          <div className="grid grid-cols-4 gap-2 text-center">
            {(
              [
                ["Thoughts", settings?.counts.thoughts],
                ["Open tasks", settings?.counts.tasksOpen],
                ["Vectors", settings?.counts.vectors],
                ["Map nodes", settings?.counts.nodes],
              ] as const
            ).map(([label, n]) => (
              <div key={label} className="rounded-xl bg-surface-2 p-3">
                <p className="text-lg font-bold tabular-nums">{n ?? "—"}</p>
                <p className="text-[10px] text-ink-3">{label}</p>
              </div>
            ))}
          </div>
          <div className="mt-3 flex items-center gap-2">
            <button
              onClick={() => void api.openDataDir().catch((e) => alert(String(e)))}
              className="rounded-lg border border-line px-3 py-1.5 text-xs text-ink-2 hover:text-ink"
            >
              Open data folder
            </button>
            <span className="truncate text-[10px] text-ink-3" title={settings?.dataDir}>
              {settings?.dataDir}
            </span>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-ink-3">
            Everything — audio, transcripts, the mind map, vectors — lives in a local
            SQLite store on this Mac. No accounts, no cloud, no telemetry.
          </p>
        </section>

        {/* updates */}
        <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-bold">Updates</h3>
              <p className="mt-1 text-xs text-ink-2">
                Nodemind checks GitHub for new versions. The download, install, and
                relaunch are automatic once you approve.
              </p>
            </div>
            <button
              onClick={checkUpdates}
              disabled={checking}
              className="shrink-0 rounded-lg bg-accent px-4 py-2 text-xs font-semibold text-white disabled:opacity-50"
            >
              {checking ? "Checking…" : "Check for updates"}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}
