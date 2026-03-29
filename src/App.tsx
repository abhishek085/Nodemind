import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useListening } from "./hooks/useListening";
import TodayView from "./views/TodayView";
import TaskListView from "./views/TaskListView";
import MeetingsView from "./views/MeetingsView";
import FogView from "./views/FogView";
import MentalMapView from "./views/MentalMapView";
import CalendarView from "./views/CalendarView";
import HistoricalNotesView from "./views/HistoricalNotesView";
import SettingsView from "./views/SettingsView";
import type { View, ActivityItem } from "./types";
import "./App.css";

const NAV_ITEMS: { id: View; label: string; emoji: string }[] = [
  { id: "today",    label: "Today",    emoji: "☀️" },
  { id: "tasks",    label: "Tasks",    emoji: "✅" },
  { id: "map",      label: "Map",      emoji: "🧠" },
  { id: "meetings", label: "Meetings", emoji: "🤝" },
  { id: "fog",      label: "Fog",      emoji: "🌫️" },
  { id: "calendar", label: "Calendar", emoji: "📅" },
  { id: "notes",    label: "Notes",    emoji: "🗂️" },
  { id: "settings", label: "Settings", emoji: "⚙️" },
];

export default function App() {
  const [view, setView] = useState<View>("today");
  const { status, transcript, language, processingFinalNote, toggleListening, setLanguage, clearTranscript } =
    useListening();
  const [ollamaOk, setOllamaOk] = useState<boolean | null>(null);
  const [modelLoaded, setModelLoaded] = useState(false);
  const [notification, setNotification] = useState<string | null>(null);
  const [triggerSummarize, setTriggerSummarize] = useState(0);
  const [activityLog, setActivityLog] = useState<ActivityItem[]>([]);
  const lastOllamaAvailableRef = useRef<boolean | null>(null);

  // Check Ollama availability on startup
  useEffect(() => {
    const check = async () => {
      try {
        const res: {
          available: boolean;
          responding?: boolean;
          latency_ms?: number;
          autostart_attempted?: boolean;
          autostart_ok?: boolean;
          autostart_error?: string | null;
        } = await invoke("check_ollama_status");
        setOllamaOk(res.available);

        const prev = lastOllamaAvailableRef.current;
        if (prev !== null && prev !== res.available) {
          flash(
            res.available
              ? "✅ Ollama is back online."
              : "⚠️ Ollama is offline. Trying to start it locally…",
          );
        }

        if (res.autostart_attempted) {
          flash(
            res.autostart_ok
              ? "🚀 Started Ollama in background."
              : `⚠️ Could not auto-start Ollama: ${String(res.autostart_error ?? "unknown error")}`,
          );
        }

        lastOllamaAvailableRef.current = res.available;
      } catch {
        setOllamaOk(false);
        if (lastOllamaAvailableRef.current !== false) {
          flash("⚠️ Ollama health check failed. LLM features are offline.");
        }
        lastOllamaAvailableRef.current = false;
      }
    };

    check();
    const id = setInterval(check, 7000);
    return () => clearInterval(id);
  }, []);

  // Poll whether the LLM model is warm in memory
  useEffect(() => {
    const check = async () => {
      try {
        const loaded: boolean = await invoke("is_model_loaded");
        setModelLoaded(loaded);
      } catch {}
    };
    check();
    const id = setInterval(check, 5000);
    return () => clearInterval(id);
  }, []);

  // Update tray icon state whenever listening status changes
  useEffect(() => {
    try {
      const label = status === "listening" ? "Listening" : status === "idle" ? "Idle" : "Off";
      invoke("update_tray_status", { status: label });
    } catch {}
  }, [status]);

  // Global backend event listeners
  useEffect(() => {
    const fns: (() => void)[] = [];
    const setup = async () => {
      fns.push(
        await listen<string>("command-detected", (e) =>
          flash(`⌨️  "${e.payload.slice(0, 60)}"`),
        ),
      );
      fns.push(await listen("meeting-started", () => flash("🤝 Meeting started")));
      fns.push(
        await listen("meeting-ended", () =>
          flash("✅ Meeting ended — generating notes…"),
        ),
      );
      fns.push(await listen("goal-created", () => flash("🎯 Goal saved!")));
      fns.push(await listen("task-created", () => flash("✅ Task saved!")));
      // Background LLM processing feedback
      fns.push(
        await listen<string>("chunk-annotated", (e) => {
          try {
            const a = JSON.parse(e.payload);
            const taskTitles: string[] = (a.tasks ?? []).map((t: { title?: string }) => t.title ?? String(t)).filter(Boolean);
            const fogSignals: string[] = a.fog_signals ?? [];
            const topics: string[] = a.topics ?? [];
            if (taskTitles.length > 0 || fogSignals.length > 0 || topics.length > 0) {
              const now = new Date().toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", hour12: false });
              setActivityLog((prev) => {
                // Merge into existing same-minute bucket, deduplicating each field
                const idx = prev.findIndex((e) => e.time === now);
                if (idx !== -1) {
                  const existing = prev[idx];
                  const merged: ActivityItem = {
                    time: now,
                    tasks: [...new Set([...existing.tasks, ...taskTitles])],
                    fog: [...new Set([...existing.fog, ...fogSignals])],
                    topics: [...new Set([...existing.topics, ...topics])],
                  };
                  const next = [...prev];
                  next[idx] = merged;
                  return next;
                }
                return [{ time: now, tasks: taskTitles, fog: fogSignals, topics }, ...prev].slice(0, 20);
              });
              const parts: string[] = [];
              if (taskTitles.length > 0) parts.push(`${taskTitles.length} task${taskTitles.length !== 1 ? "s" : ""}`);
              if (fogSignals.length > 0) parts.push(`${fogSignals.length} fog signal${fogSignals.length !== 1 ? "s" : ""}`);
              if (parts.length > 0) flash(`🔍 Captured: ${parts.join(", ")}`);
            }
          } catch {}
        }),
      );
      fns.push(
        await listen<string>("command-parsed", (e) => {
          try {
            const c = JSON.parse(e.payload);
            const intent = c.intent ?? "Unknown";
            if (intent !== "Unknown") flash(`🎯 Command: ${intent}`);
          } catch {}
        }),
      );
      fns.push(
        await listen<string>("error", (e) => {
          flash(`⚠️ ${String(e.payload).slice(0, 80)}`);
        }),
      );
      fns.push(
        await listen<string>("mental-loop-detected", (e) => {
          try {
            const payload = JSON.parse(e.payload);
            const subject = String(payload.subject ?? "the same topic");
            flash(`🔁 Loop alert: You may be circling on ${subject}`);
          } catch {
            flash("🔁 Loop alert: You may be circling on the same topic");
          }
        }),
      );
      // Tray menu events
      fns.push(await listen("tray-toggle-listening", () => toggleListening()));
      fns.push(
        await listen("tray-summarize", () => {
          setView("today");
          setTriggerSummarize((n) => n + 1);
        }),
      );
    };
    setup();
    return () => fns.forEach((u) => u());
  }, []);

  const flash = (msg: string) => {
    setNotification(msg);
    setTimeout(() => setNotification(null), 3500);
  };

  const listeningCls =
    status === "listening"
      ? "status-listening"
      : status === "idle"
        ? "status-idle"
        : "status-off";

  return (
    <div className="app-shell">
      {/* ── Titlebar ───────────────────────────────────── */}
      <header className="titlebar">
        <div className="titlebar-brand">
          <img src="/nodemind_icon.svg" alt="Nodemind" className="titlebar-logo" />
          <div className="app-wordmark">
            <span className="app-name">
              <span className="app-name-node">Node</span>
              <span className="app-name-mind">mind</span>
            </span>
            <span className="app-byline">by Nokast</span>
          </div>
        </div>
        <div className="titlebar-right">
          {ollamaOk === false && (
            <span className="badge-warn" title="Ollama not running — LLM features offline">
              No LLM
            </span>
          )}
          {ollamaOk === true && (
            <span
              className={modelLoaded ? "badge-ok" : "badge-model-cold"}
              title={modelLoaded ? "LLM model is loaded in memory" : "LLM available but model not yet warmed up — go to Settings to preload it"}
            >
              {modelLoaded ? "Model ✓" : "Model ○"}
            </span>
          )}
          <button
            className={`mic-pill ${listeningCls}`}
            onClick={toggleListening}
            title={status === "listening" ? "Stop listening" : "Start listening"}
          >
            <span className="mic-dot" />
            {status === "listening" ? "Listening" : status === "idle" ? "Idle" : "Off"}
          </button>
          <button
            className={`lang-toggle${language === "hi" ? " lang-hi" : ""}`}
            onClick={() => setLanguage(language === "en" ? "hi" : "en")}
            title="Toggle language"
          >
            {language === "en" ? "EN" : "HI"}
          </button>
        </div>
      </header>

      {/* ── Toast ──────────────────────────────────────── */}
      {notification && <div className="toast-bar">{notification}</div>}

      {/* ── Live transcript strip ───────────────────────── */}
      {status === "listening" && transcript && (
        <div className="live-bar">
          <span className="live-dot" />
          <span className="live-text">{transcript.slice(-140)}</span>
          <button className="clear-btn" onClick={clearTranscript}>
            clear
          </button>
        </div>
      )}

      {processingFinalNote && (
        <div className="processing-bar">
          <span className="processing-dot" />
          <span className="processing-text">Processing final note in background...</span>
        </div>
      )}

      {/* ── Main layout ─────────────────────────────────── */}
      <div className="main-layout">
        <nav className="sidebar">
          {NAV_ITEMS.map((item) => (
            <button
              key={item.id}
              className={`nav-btn${view === item.id ? " nav-active" : ""}`}
              onClick={() => setView(item.id)}
            >
              <span className="nav-emoji">{item.emoji}</span>
              <span className="nav-label">{item.label}</span>
            </button>
          ))}
          <div className="sidebar-spacer" />
          <div className="sidebar-brand">
            <img src="/nodemind_icon.svg" alt="Nodemind" className="nokast-logo" />
          </div>
        </nav>

        <main className="content-pane">
          {view === "today" && (
            <TodayView
              transcript={transcript}
              listeningStatus={status}
              processingFinalNote={processingFinalNote}
              language={language}
              triggerSummarize={triggerSummarize}
              activityLog={activityLog}
            />
          )}
          {view === "tasks" && <TaskListView />}
          {view === "map" && <MentalMapView />}
          {view === "meetings" && <MeetingsView />}
          {view === "fog" && <FogView />}
          {view === "calendar" && <CalendarView />}
          {view === "notes" && <HistoricalNotesView onOpenToday={() => setView("today")} />}
          {view === "settings" && <SettingsView />}
        </main>
      </div>
    </div>
  );
}
