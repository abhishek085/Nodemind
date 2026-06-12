import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GraphNode, GraphEdge, DriftAlert, Task } from "../types";
import DriftAlertCard from "../components/DriftAlertCard";
import MomentumBar from "../components/MomentumBar";

const NODE_TYPE_COLORS: Record<string, string> = {
  project: "#818cf8",
  goal: "#34d399",
  person: "#fb923c",
  task: "#38bdf8",
  topic: "#a78bfa",
  fog_pattern: "#f87171",
  fog: "#f87171",
  idea: "#fbbf24",
  self: "#e879f9",
};

function initials(name: string): string {
  return name
    .split(/\s+/)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .slice(0, 2)
    .join("");
}

interface DashboardViewProps {
  nodes: GraphNode[];
  edges?: GraphEdge[];
  driftAlerts: DriftAlert[];
  momentumScores: Record<string, number>;
  tasks: Task[];
  onNavigateToGraph: (nodeId?: string) => void;
}

interface PersonNode {
  id: string;
  label: string;
  last_mentioned_at?: string | null;
}

const DashboardView: React.FC<DashboardViewProps> = ({
  nodes,
  driftAlerts,
  momentumScores,
  tasks,
  onNavigateToGraph,
}) => {
  const [fogSentence, setFogSentence] = useState<string>("");
  const [dismissedAlerts, setDismissedAlerts] = useState<Set<string>>(new Set());
  const [unprocessedCount, setUnprocessedCount] = useState(0);
  const [runningModels, setRunningModels] = useState<string[]>([]);
  const [processing, setProcessing] = useState(false);
  const [processMsg, setProcessMsg] = useState<string | null>(null);
  const [batchSchedule, setBatchSchedule] = useState<{
    last_ran_at: string;
    seconds_until_next: number;
    last_ran_ago_secs: number;
    is_overdue: boolean;
    batch_interval_secs: number;
  } | null>(null);
  // Live countdown ticks every second from the fetched schedule
  const [countdown, setCountdown] = useState(0);

  // Poll unprocessed count, running models, and batch schedule
  useEffect(() => {
    const poll = async () => {
      try {
        const count: number = await invoke("get_unprocessed_notes_count");
        setUnprocessedCount(count);
      } catch {}
      try {
        const res: { running_models?: string[] } = await invoke("check_ollama_status");
        setRunningModels(res.running_models ?? []);
      } catch {}
      try {
        const sched = await invoke<typeof batchSchedule>("get_batch_schedule");
        setBatchSchedule(sched);
        setCountdown(sched?.seconds_until_next ?? 0);
      } catch {}
    };
    poll();
    const id = setInterval(poll, 30_000);
    return () => clearInterval(id);
  }, []);

  // Live countdown — ticks every second between remote polls
  useEffect(() => {
    const id = setInterval(() => {
      setCountdown((c) => Math.max(0, c - 1));
    }, 1000);
    return () => clearInterval(id);
  }, []);

  const handleProcessNow = async () => {
    setProcessing(true);
    setProcessMsg(null);
    try {
      const msg: string = await invoke("refresh_suggestions");
      setProcessMsg(msg);
      const count: number = await invoke("get_unprocessed_notes_count");
      setUnprocessedCount(count);
      // Refresh schedule after manual trigger
      const sched = await invoke<typeof batchSchedule>("get_batch_schedule");
      setBatchSchedule(sched);
      setCountdown(sched?.seconds_until_next ?? 0);
    } catch (e) {
      setProcessMsg(`Error: ${String(e)}`);
    } finally {
      setProcessing(false);
      setTimeout(() => setProcessMsg(null), 5000);
    }
  };

  // Derive fog sentence from fog signal nodes
  useEffect(() => {
    const fogNodes = nodes.filter(
      (n) => n.node_type === "fog_pattern" || n.node_type === "fog"
    );
    const recent24h = fogNodes.filter((n) => {
      const ts = n.last_mentioned_at ?? n.created_at;
      return Date.now() - new Date(ts).getTime() < 86400000;
    });
    if (recent24h.length === 0) {
      setFogSentence("");
    } else {
      const topics = [...new Set(recent24h.map((n) => n.label))].slice(0, 3);
      if (topics.length === 1) {
        setFogSentence(`You've been returning to "${topics[0]}" today.`);
      } else {
        setFogSentence(
          `You've been scattered across ${topics.length} topics: ${topics.join(", ")}.`
        );
      }
    }
  }, [nodes]);

  // Top 3 projects by momentum
  const projectNodes = nodes.filter((n) => n.node_type === "project");
  const topProjects = [...projectNodes]
    .map((n) => ({ node: n, score: momentumScores[n.id] ?? 0 }))
    .sort((a, b) => b.score - a.score)
    .slice(0, 3);

  // Recent extractions (last 24h, ordered newest first)
  const cutoff24h = Date.now() - 86400000;
  const recentNodes = [...nodes]
    .filter((n) => {
      const ts = n.last_mentioned_at ?? n.created_at;
      return new Date(ts).getTime() > cutoff24h;
    })
    .sort((a, b) => {
      const ta = new Date(a.last_mentioned_at ?? a.created_at).getTime();
      const tb = new Date(b.last_mentioned_at ?? b.created_at).getTime();
      return tb - ta;
    })
    .slice(0, 12);

  // People recently mentioned (last 48h)
  const cutoff48h = Date.now() - 2 * 86400000;
  const recentPeople: PersonNode[] = nodes
    .filter((n) => n.node_type === "person")
    .filter((n) => {
      const ts = n.last_mentioned_at ?? n.created_at;
      return new Date(ts).getTime() > cutoff48h;
    })
    .sort((a, b) => {
      const ta = new Date(a.last_mentioned_at ?? a.created_at).getTime();
      const tb = new Date(b.last_mentioned_at ?? b.created_at).getTime();
      return tb - ta;
    })
    .slice(0, 6);

  // Upcoming tasks (max 7, by urgency approximated by creation order)
  const pendingTasks = tasks.filter((t) => !t.done).slice(0, 7);

  // Drift alerts not yet dismissed
  const visibleAlerts = driftAlerts
    .filter((a) => !dismissedAlerts.has(a.goal_id))
    .sort((a, b) => b.drift_score - a.drift_score)
    .slice(0, 1);

  const handleKeep = (id: string) => setDismissedAlerts((s) => new Set([...s, id]));
  const handleArchive = (id: string) => setDismissedAlerts((s) => new Set([...s, id]));
  const handleSnooze = (id: string) => setDismissedAlerts((s) => new Set([...s, id]));

  return (
    <div style={{ height: "100%", overflowY: "auto", background: "#0f172a", color: "#fff", padding: "0" }}>

      {/* Pipeline status bar */}
      <PipelineBar
        unprocessedCount={unprocessedCount}
        runningModels={runningModels}
        processing={processing}
        processMsg={processMsg}
        countdown={countdown}
        batchSchedule={batchSchedule}
        onProcessNow={handleProcessNow}
      />

      {/* 1. Drift Alert Strip */}
      {visibleAlerts.length > 0 && (
        <div style={{ padding: "12px 20px", borderBottom: "1px solid rgba(255,255,255,0.06)" }}>
          {visibleAlerts.map((a) => (
            <DriftAlertCard
              key={a.goal_id}
              alert={a}
              onKeep={handleKeep}
              onArchive={handleArchive}
              onSnooze={handleSnooze}
            />
          ))}
        </div>
      )}

      <div style={{ padding: "20px", display: "flex", flexDirection: "column", gap: "24px" }}>

        {/* 2. Today's Fog Signal */}
        {fogSentence && (
          <Section title="Fog Signal">
            <p style={{ fontSize: "14px", color: "rgba(255,255,255,0.75)", margin: 0, lineHeight: 1.6 }}>
              {fogSentence}
            </p>
          </Section>
        )}

        {/* 3. Momentum */}
        {topProjects.length > 0 && (
          <Section title="Momentum">
            {topProjects.map(({ node, score }, i) => (
              <MomentumBar key={node.id} label={node.label} score={score} rank={i + 1} />
            ))}
          </Section>
        )}

        {/* 4. Recent Extractions Feed */}
        {recentNodes.length > 0 && (
          <Section title="Recent Extractions">
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px" }}>
              {recentNodes.map((n) => (
                <button
                  key={n.id}
                  onClick={() => onNavigateToGraph(n.id)}
                  style={{
                    padding: "3px 10px",
                    borderRadius: "12px",
                    border: `1px solid ${NODE_TYPE_COLORS[n.node_type] ?? "#6b7280"}`,
                    background: "transparent",
                    color: NODE_TYPE_COLORS[n.node_type] ?? "#9ca3af",
                    fontSize: "12px",
                    cursor: "pointer",
                    display: "flex",
                    alignItems: "center",
                    gap: "4px",
                  }}
                >
                  <span>{n.label}</span>
                  <span style={{ fontSize: "10px", opacity: 0.6 }}>→</span>
                </button>
              ))}
            </div>
          </Section>
        )}

        {/* 5. Upcoming Tasks */}
        {pendingTasks.length > 0 && (
          <Section title="Upcoming Tasks">
            <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
              {pendingTasks.map((t) => (
                <div
                  key={t.id}
                  style={{
                    padding: "8px 12px",
                    background: "rgba(255,255,255,0.04)",
                    borderRadius: "6px",
                    borderLeft: "2px solid #38bdf8",
                  }}
                >
                  <div style={{ fontSize: "13px", color: "rgba(255,255,255,0.9)" }}>{t.title}</div>
                  {t.project && (
                    <div style={{ fontSize: "11px", color: "rgba(255,255,255,0.4)", marginTop: "2px" }}>
                      {t.project}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </Section>
        )}

        {/* 6. People Recently Mentioned */}
        {recentPeople.length > 0 && (
          <Section title="People">
            <div style={{ display: "flex", gap: "12px", flexWrap: "wrap" }}>
              {recentPeople.map((p) => (
                <button
                  key={p.id}
                  onClick={() => onNavigateToGraph(p.id)}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "center",
                    gap: "4px",
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                  }}
                >
                  <div
                    style={{
                      width: "36px",
                      height: "36px",
                      borderRadius: "50%",
                      background: "#fb923c22",
                      border: "1px solid #fb923c55",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      fontSize: "13px",
                      fontWeight: 600,
                      color: "#fb923c",
                    }}
                  >
                    {initials(p.label)}
                  </div>
                  <span style={{ fontSize: "11px", color: "rgba(255,255,255,0.55)", maxWidth: "44px", textOverflow: "ellipsis", overflow: "hidden", whiteSpace: "nowrap" }}>
                    {p.label.split(" ")[0]}
                  </span>
                </button>
              ))}
            </div>
          </Section>
        )}

        {/* Empty state */}
        {nodes.length === 0 && tasks.length === 0 && (
          <div style={{ textAlign: "center", color: "rgba(255,255,255,0.3)", paddingTop: "60px" }}>
            <div style={{ fontSize: "36px", marginBottom: "12px" }}>◎</div>
            <div style={{ fontSize: "14px" }}>Start a voice session to build your knowledge graph.</div>
          </div>
        )}

      </div>
    </div>
  );
};

function formatDuration(secs: number): string {
  if (secs <= 0) return "now";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m.toString().padStart(2, "0")}m`;
  if (m > 0) return `${m}m ${s.toString().padStart(2, "0")}s`;
  return `${s}s`;
}

function formatAgo(secs: number): string {
  if (secs < 0) return "never";
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m ago`;
}

function PipelineBar({
  unprocessedCount,
  runningModels,
  processing,
  processMsg,
  countdown,
  batchSchedule,
  onProcessNow,
}: {
  unprocessedCount: number;
  runningModels: string[];
  processing: boolean;
  processMsg: string | null;
  countdown: number;
  batchSchedule: {
    last_ran_at: string;
    seconds_until_next: number;
    last_ran_ago_secs: number;
    is_overdue: boolean;
    batch_interval_secs: number;
  } | null;
  onProcessNow: () => void;
}) {
  const BATCH_MODEL = "qwen3.5:9b";
  const REALTIME_MODEL = "gemma4:e4b";

  const batchRunning = runningModels.some((m) => m.includes("qwen") || m.includes(BATCH_MODEL));
  const realtimeRunning = runningModels.some((m) => m.includes("gemma") || m.includes(REALTIME_MODEL));

  const isOverdue = batchSchedule?.is_overdue ?? false;
  const neverRan = !batchSchedule?.last_ran_at;

  // Countdown display: live ticking between 30s polls
  const countdownLabel = (() => {
    if (processing) return "Processing…";
    if (neverRan) return "Not run yet";
    if (isOverdue) return "Overdue";
    return `Next in ${formatDuration(countdown)}`;
  })();

  const countdownColor = processing
    ? "#818cf8"
    : isOverdue
    ? "#fbbf24"
    : neverRan
    ? "rgba(255,255,255,0.3)"
    : "rgba(255,255,255,0.5)";

  return (
    <div style={{
      display: "flex",
      alignItems: "center",
      gap: "12px",
      padding: "7px 20px",
      background: "rgba(255,255,255,0.03)",
      borderBottom: "1px solid rgba(255,255,255,0.07)",
      fontSize: "12px",
      flexWrap: "wrap",
    }}>
      {/* Unprocessed notes indicator */}
      <span style={{ color: unprocessedCount > 0 ? "#fbbf24" : "rgba(255,255,255,0.35)" }}>
        {unprocessedCount > 0
          ? `${unprocessedCount} note${unprocessedCount > 1 ? "s" : ""} pending`
          : "Notes up to date"}
      </span>

      {/* Batch timer */}
      <span
        title={
          batchSchedule?.last_ran_at
            ? `Last batch: ${formatAgo(batchSchedule.last_ran_ago_secs)}. Runs every 4h. Persisted in DB — survives app restart.`
            : "Batch has not run yet this session."
        }
        style={{
          color: countdownColor,
          display: "flex",
          alignItems: "center",
          gap: "4px",
          cursor: "default",
        }}
      >
        <span style={{ fontSize: "10px" }}>⏱</span>
        {countdownLabel}
        {batchSchedule?.last_ran_ago_secs !== undefined && batchSchedule.last_ran_ago_secs >= 0 && !processing && (
          <span style={{ opacity: 0.5, fontSize: "11px" }}>
            · last {formatAgo(batchSchedule.last_ran_ago_secs)}
          </span>
        )}
      </span>

      {/* Process Now button */}
      {(unprocessedCount > 0 || isOverdue) && (
        <button
          onClick={onProcessNow}
          disabled={processing}
          style={{
            padding: "2px 10px",
            borderRadius: "10px",
            border: "1px solid #818cf8",
            background: "transparent",
            color: "#818cf8",
            fontSize: "11px",
            cursor: processing ? "default" : "pointer",
            opacity: processing ? 0.5 : 1,
          }}
        >
          {processing ? "Processing…" : "Process Now"}
        </button>
      )}

      {/* Model pills */}
      <div style={{ display: "flex", gap: "6px", marginLeft: "auto", alignItems: "center" }}>
        <ModelPill label="Batch (qwen3.5:9b)" active={batchRunning} color="#818cf8" />
        <ModelPill label="Realtime (gemma4:e4b)" active={realtimeRunning} color="#34d399" />
      </div>

      {/* Feedback message */}
      {processMsg && (
        <span style={{ color: "rgba(255,255,255,0.5)", fontSize: "11px", width: "100%" }}>
          {processMsg}
        </span>
      )}
    </div>
  );
}

function ModelPill({ label, active, color }: { label: string; active: boolean; color: string }) {
  return (
    <span style={{
      display: "inline-flex",
      alignItems: "center",
      gap: "5px",
      padding: "2px 8px",
      borderRadius: "10px",
      border: `1px solid ${active ? color : "rgba(255,255,255,0.12)"}`,
      color: active ? color : "rgba(255,255,255,0.3)",
      fontSize: "11px",
      background: active ? `${color}18` : "transparent",
    }}>
      <span style={{
        width: "6px",
        height: "6px",
        borderRadius: "50%",
        background: active ? color : "rgba(255,255,255,0.2)",
        display: "inline-block",
        boxShadow: active ? `0 0 5px ${color}` : "none",
      }} />
      {label}
    </span>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{
        fontSize: "11px",
        color: "rgba(255,255,255,0.35)",
        textTransform: "uppercase",
        letterSpacing: "0.08em",
        fontWeight: 600,
        marginBottom: "10px",
      }}>
        {title}
      </div>
      {children}
    </div>
  );
}

export default DashboardView;
