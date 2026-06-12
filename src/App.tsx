import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useListening } from "./hooks/useListening";
import { useGraphData } from "./hooks/useGraphData";
import DashboardView from "./views/DashboardView";
import KnowledgeGraphView from "./views/KnowledgeGraphView";
import FocusMode from "./views/FocusMode";
import SettingsView from "./views/SettingsView";
import Sidebar from "./components/Sidebar";
import type { View, ActivityItem } from "./types";
import "./App.css";

// NAV_ITEMS no longer needed — Sidebar component owns the nav definition

// ─── Auto-trigger Focus Mode ──────────────────────────────────────────────────
function shouldAutoTriggerFocus(): boolean {
  const now = new Date();
  const hour = now.getHours();
  const lastActivity = localStorage.getItem("nodemind_last_activity");
  const gapHours = lastActivity
    ? (Date.now() - parseInt(lastActivity)) / 3600000
    : 99;
  return hour < 10 || gapHours >= 4;
}

export default function App() {
  const savedView = (localStorage.getItem("nodemind_last_view") as View | null);
  const [view, setView] = useState<View>(savedView ?? "dashboard");
  const [showFocus, setShowFocus] = useState(false);

  const { status, transcript, language, processingFinalNote, toggleListening, setLanguage, clearTranscript } =
    useListening();
  const { graphData, driftAlerts, momentumScores, recentTasks, refresh: refreshGraph } = useGraphData();

  const [activeFocusProject, setActiveFocusProject] = useState<string | null>(
    () => localStorage.getItem("nodemind_focus_project")
  );

  const [ollamaOk, setOllamaOk] = useState<boolean | null>(null);
  const [modelLoaded, setModelLoaded] = useState(false);
  const [notification, setNotification] = useState<string | null>(null);
  const [, setActivityLog] = useState<ActivityItem[]>([]);
  const lastOllamaAvailableRef = useRef<boolean | null>(null);
  // nodeId to navigate to in graph view - used when navigating from dashboard
  const [, setGraphNavTarget] = useState<string | undefined>(undefined);

  // Persist last view
  const changeView = (v: View) => {
    setView(v);
    localStorage.setItem("nodemind_last_view", v);
    localStorage.setItem("nodemind_last_activity", String(Date.now()));
  };

  // Navigate to a node in the graph (called from Dashboard)
  const navigateToGraph = (nodeId?: string) => {
    setGraphNavTarget(nodeId);
    changeView("graph");
  };

  // Auto-trigger Focus Mode check
  useEffect(() => {
    if (shouldAutoTriggerFocus()) {
      setShowFocus(true);
    }
    localStorage.setItem("nodemind_last_activity", String(Date.now()));
  }, []);

  // Focus mode: trigger when user clicks nav item
  const handleFocusNav = () => {
    setShowFocus(true);
  };

  const handleStartFocus = (projectLabel: string) => {
    setActiveFocusProject(projectLabel);
    localStorage.setItem("nodemind_focus_project", projectLabel);
  };

  // Check Ollama availability on startup
  useEffect(() => {
    const check = async () => {
      try {
        const res: {
          available: boolean;
          autostart_attempted?: boolean;
          autostart_ok?: boolean;
          autostart_error?: string | null;
        } = await invoke("check_ollama_status");
        setOllamaOk(res.available);

        const prev = lastOllamaAvailableRef.current;
        if (prev !== null && prev !== res.available) {
          flash(res.available ? "✅ Ollama is back online." : "⚠️ Ollama is offline. Trying to start it locally…");
        }
        if (res.autostart_attempted) {
          flash(res.autostart_ok ? "🚀 Started Ollama in background." : `⚠️ Could not auto-start Ollama: ${String(res.autostart_error ?? "unknown")}`);
        }
        lastOllamaAvailableRef.current = res.available;
      } catch {
        setOllamaOk(false);
        if (lastOllamaAvailableRef.current !== false) flash("⚠️ Ollama health check failed. LLM features are offline.");
        lastOllamaAvailableRef.current = false;
      }
    };
    check();
    const id = setInterval(check, 7000);
    return () => clearInterval(id);
  }, []);

  // Poll model loaded state
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

  // Update tray icon
  useEffect(() => {
    try {
      invoke("update_tray_status", { status: status === "listening" ? "Listening" : status === "idle" ? "Idle" : "Off" });
    } catch {}
  }, [status]);

  // Global backend event listeners
  useEffect(() => {
    const fns: (() => void)[] = [];
    const setup = async () => {
      fns.push(await listen<string>("command-detected", (e) => flash(`⌨️  "${e.payload.slice(0, 60)}"`)));
      fns.push(await listen("meeting-started", () => flash("🤝 Meeting started")));
      fns.push(await listen("meeting-ended", () => flash("✅ Meeting ended — generating notes…")));
      fns.push(await listen("goal-created", () => flash("🎯 Goal saved!")));
      fns.push(await listen("task-created", () => flash("✅ Task saved!")));
      fns.push(await listen("mental-map-updated", () => {
        flash("🧠 Knowledge graph updated");
        refreshGraph();
      }));
      fns.push(await listen<string>("chunk-annotated", (e) => {
        try {
          const a = JSON.parse(e.payload);
          const taskTitles: string[] = (a.tasks ?? []).map((t: { title?: string }) => t.title ?? String(t)).filter(Boolean);
          const fogSignals: string[] = a.fog_signals ?? [];
          const topics: string[] = a.topics ?? [];
          if (taskTitles.length > 0 || fogSignals.length > 0 || topics.length > 0) {
            const now = new Date().toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", hour12: false });
            setActivityLog((prev) => {
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
          }
        } catch {}
      }));
      fns.push(await listen<string>("error", (e) => flash(`⚠️ ${String(e.payload).slice(0, 80)}`)));
      fns.push(await listen("tray-toggle-listening", () => toggleListening()));
    };
    setup();
    return () => fns.forEach((u) => u());
  }, [refreshGraph, toggleListening]);

  const flash = (msg: string) => {
    setNotification(msg);
    setTimeout(() => setNotification(null), 3500);
  };

  const listeningCls = status === "listening" ? "status-listening" : status === "idle" ? "status-idle" : "status-off";

  return (
    <div className="app-shell">
      {/* ── Titlebar ─────────────────────────────────────── */}
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
            <span className="badge-warn" title="Ollama not running — LLM features offline">No LLM</span>
          )}
          {ollamaOk === true && (
            <span className={modelLoaded ? "badge-ok" : "badge-model-cold"} title={modelLoaded ? "LLM model loaded" : "Model not yet warm"}>
              {modelLoaded ? "Model ✓" : "Model ○"}
            </span>
          )}
          <button className={`mic-pill ${listeningCls}`} onClick={toggleListening} title={status === "listening" ? "Stop listening" : "Start listening"}>
            <span className="mic-dot" />
            {status === "listening" ? "Listening" : status === "idle" ? "Idle" : "Off"}
          </button>
          <button className={`lang-toggle${language === "hi" ? " lang-hi" : ""}`} onClick={() => setLanguage(language === "en" ? "hi" : "en")} title="Toggle language">
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
          <button className="clear-btn" onClick={clearTranscript}>clear</button>
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
        <Sidebar
          activeView={view}
          onNav={changeView}
          onFocus={handleFocusNav}
          activeFocusProject={activeFocusProject}
        />

        <main className="content-pane">
          {view === "dashboard" && (
            <DashboardView
              nodes={graphData.nodes}
              edges={graphData.edges}
              driftAlerts={driftAlerts}
              momentumScores={momentumScores}
              tasks={recentTasks}
              onNavigateToGraph={navigateToGraph}
            />
          )}
          {view === "graph" && (
            <KnowledgeGraphView
              nodes={graphData.nodes}
              edges={graphData.edges}
              momentumScores={momentumScores}
            />
          )}
          {view === "settings" && <SettingsView />}
        </main>
      </div>

      {/* ── Focus Mode overlay ────────────────────────── */}
      {showFocus && (
        <FocusMode
          nodes={graphData.nodes}
          driftAlerts={driftAlerts}
          momentumScores={momentumScores}
          tasks={recentTasks}
          onStartFocus={handleStartFocus}
          onDismiss={() => {
            setShowFocus(false);
            localStorage.setItem("nodemind_last_activity", String(Date.now()));
          }}
        />
      )}
    </div>
  );
}
