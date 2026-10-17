// App shell: sidebar + status bar + the active view.
//
// Gates the whole app on engine readiness — before the local models exist,
// only the SetupScreen is reachable.

import { useCallback, useEffect, useState } from "react";

import * as api from "./lib/api";
import { applyModelProgress } from "./lib/api";
import SetupScreen from "./components/SetupScreen";
import Sidebar, { type ViewId } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import UpdateBanner from "./components/UpdateBanner";
import CaptureView from "./views/CaptureView";
import GraphView from "./views/GraphView";
import SearchView from "./views/SearchView";
import TodayView from "./views/TodayView";
import SettingsView from "./views/SettingsView";
import type { EngineStatus, ModelProgress, Settings, UpdateInfo } from "./lib/types";

const TITLES: Record<ViewId, string> = {
  capture: "Capture",
  search: "Search",
  graph: "Mind Map",
  today: "Today",
  settings: "Settings",
};

export default function App() {
  const [view, setView] = useState<ViewId>("capture");
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [setupDone, setSetupDone] = useState(false);
  const [mapFocus, setMapFocus] = useState<string | null>(null); // thought to highlight

  const bump = useCallback(() => setRefreshKey((k) => k + 1), []);

  // ── engine status: initial fetch + live events + download progress ───────
  useEffect(() => {
    void api.engineStatus().then(setStatus).catch(() => undefined);
    const unStatus = api.on<EngineStatus>("engine-status", (s) => {
      setStatus(s);
      if (s.phase === "ready") setSetupDone(true);
    });
    const unProgress = api.on<ModelProgress>("model-progress", (p) =>
      setStatus((cur) => (cur ? applyModelProgress(cur, p) : cur)),
    );
    return () => {
      void unStatus.then((f) => f());
      void unProgress.then((f) => f());
    };
  }, []);

  // ── settings + first-launch update check ─────────────────────────────────
  useEffect(() => {
    void api
      .getSettings()
      .then(setSettings)
      .catch(() => undefined);
    const un = api.on<{ language: string }>("settings-updated", (e) =>
      setSettings((s) => (s ? { ...s, language: e.language } : s)),
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (status?.phase !== "ready") return;
    void api
      .checkForUpdates()
      .then(setUpdate)
      .catch(() => undefined);
  }, [status?.phase]);

  // ── capture events: refresh views that show derived data ─────────────────
  useEffect(() => {
    const un = api.on<unknown>("capture-complete", () => bump());
    return () => {
      void un.then((f) => f());
    };
  }, [bump]);

  // ── ⌘K jumps to search from anywhere in the app ─────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setView("search");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ── the menu-bar popover opens this window on a specific view ────────────
  useEffect(() => {
    const un = api.on<ViewId>("navigate", (v) => setView(v));
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // While the engine isn't ready, nothing else is usable.
  const needsSetup = status !== null && status.phase === "setup" && !setupDone;
  const failed = status?.phase === "failed";

  return (
    <div className="flex h-full flex-col bg-bg">
      <StatusBar status={status} viewTitle={TITLES[view]} />
      {update && <UpdateBanner update={update} onDismiss={() => setUpdate(null)} />}

      {needsSetup ? (
        <SetupScreen status={status} onReady={() => setSetupDone(true)} />
      ) : (
        <div className="flex min-h-0 flex-1">
          <Sidebar view={view} onNavigate={setView} />
          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            {view === "capture" && <CaptureView settings={settings} onCaptured={bump} />}
            {view === "search" && (
              <SearchView
                onShowInMap={(id) => {
                  setMapFocus(id);
                  setView("graph");
                }}
              />
            )}
            {view === "graph" && <GraphView refreshKey={refreshKey} focusId={mapFocus} />}
            {view === "today" && <TodayView refreshKey={refreshKey} />}
            {view === "settings" && (
              <SettingsView settings={settings} status={status} onSettings={setSettings} />
            )}
          </main>
        </div>
      )}

      {failed && (
        <div className="no-drag border-t border-danger/30 bg-danger/10 px-4 py-1.5 text-[11px] text-danger">
          {status?.error ?? "Engine error"} — open Settings to retry.
        </div>
      )}
    </div>
  );
}
