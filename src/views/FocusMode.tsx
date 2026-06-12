import React, { useEffect, useState } from "react";
import type { GraphNode, DriftAlert, Task } from "../types";
import MomentumBar from "../components/MomentumBar";

interface FocusModeProps {
  nodes: GraphNode[];
  driftAlerts?: DriftAlert[];
  momentumScores: Record<string, number>;
  tasks: Task[];
  onDismiss: () => void;
  onStartFocus?: (projectLabel: string) => void;
}

interface FocusCandidate {
  node: GraphNode;
  score: number;
  tasks: Task[];
}

function urgencyWeight(urgency: string | undefined): number {
  if (urgency === "high") return 1.0;
  if (urgency === "medium") return 0.6;
  return 0.3;
}

// Derive urgency from Task fields since Task has no urgency column
function taskUrgency(t: Task): string {
  if (t.due_hint) return "high";
  const ageMs = Date.now() - new Date(t.created_at).getTime();
  if (ageMs < 7 * 86400000) return "medium";
  return "low";
}

const FocusMode: React.FC<FocusModeProps> = ({
  nodes,
  momentumScores,
  tasks,
  onDismiss,
  onStartFocus,
}) => {
  const [candidateIndex, setCandidateIndex] = useState(0);
  const [started, setStarted] = useState(false);
  const [fogLines, setFogLines] = useState<string>("");

  // Build fog summary from fog nodes
  useEffect(() => {
    const yesterday = Date.now() - 86400000;
    const fogNodes = nodes.filter(
      (n) => (n.node_type === "fog_pattern" || n.node_type === "fog") &&
             new Date(n.last_mentioned_at ?? n.created_at).getTime() > yesterday
    );
    const topics = [...new Set(fogNodes.map((n) => n.label))].slice(0, 2);
    if (topics.length >= 2) {
      setFogLines(`You were scattered between ${topics[0]} and ${topics[1]}. Today might be a good day to pick one.`);
    } else if (topics.length === 1) {
      setFogLines(`You kept returning to ${topics[0]} yesterday.`);
    } else {
      setFogLines("No significant fog detected in the last 24 hours.");
    }
  }, [nodes]);

  // Build ranked candidates from project nodes
  const pendingTasks = tasks.filter((t) => !t.done);
  const projectNodes = nodes.filter((n) => n.node_type === "project");

  const candidates: FocusCandidate[] = projectNodes
    .map((node) => {
      const nodeTasks = pendingTasks.filter(
        (t) => t.project && node.label.toLowerCase().includes(t.project.toLowerCase())
      );
      const avgUrgency =
        nodeTasks.length > 0
          ? nodeTasks.reduce((sum, t) => {
              return sum + urgencyWeight(taskUrgency(t));
            }, 0) / nodeTasks.length
          : 0.3;
      const momentum = momentumScores[node.id] ?? 0;
      const score = momentum * avgUrgency;
      return { node, score, tasks: nodeTasks.slice(0, 3) };
    })
    .filter((c) => c.score > 0 || c.tasks.length > 0)
    .sort((a, b) => b.score - a.score);

  const current = candidates[candidateIndex] ?? null;
  const hasPrev = candidateIndex > 0;
  const hasNext = candidateIndex < Math.min(candidates.length - 1, 2);

  if (started) {
    return (
      <div style={overlayStyle}>
        <div style={innerStyle}>
          <div style={{ textAlign: "center", paddingTop: "48px" }}>
            <div style={{ fontSize: "48px", marginBottom: "12px" }}>▶</div>
            <div style={{ fontSize: "20px", fontWeight: 700 }}>
              Focus started: {current?.node.label ?? "Open work"}
            </div>
            <div style={{ fontSize: "13px", color: "rgba(255,255,255,0.5)", marginTop: "8px" }}>
              Good luck. Come back when you need new context.
            </div>
            <button
              onClick={() => {
                setStarted(false);
                onDismiss();
              }}
              style={{ ...btnStyle("#ffffff22", "#fff"), marginTop: "32px" }}
            >
              Done
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div style={overlayStyle}>
      <div style={innerStyle}>

        {/* Header */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "32px" }}>
          <span style={{ fontSize: "12px", color: "rgba(255,255,255,0.35)", textTransform: "uppercase", letterSpacing: "0.08em" }}>
            Focus Mode
          </span>
          <button
            onClick={onDismiss}
            style={{ background: "none", border: "none", color: "rgba(255,255,255,0.4)", fontSize: "16px", cursor: "pointer" }}
          >
            ✕
          </button>
        </div>

        {/* Panel 1: Today's Focus Pick */}
        <div style={{ marginBottom: "28px" }}>
          <Label>Today's Focus</Label>
          {current ? (
            <div style={{ padding: "16px", background: "rgba(255,255,255,0.05)", borderRadius: "12px", border: "1px solid rgba(255,255,255,0.1)" }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", marginBottom: "10px" }}>
                <div style={{ fontSize: "22px", fontWeight: 700 }}>{current.node.label}</div>
                <div style={{ display: "flex", gap: "6px" }}>
                  {hasPrev && (
                    <button onClick={() => setCandidateIndex((i) => i - 1)} style={arrowBtn}>←</button>
                  )}
                  {hasNext && (
                    <button onClick={() => setCandidateIndex((i) => i + 1)} style={arrowBtn}>→</button>
                  )}
                </div>
              </div>
              <div style={{ marginBottom: "12px" }}>
                <MomentumBar label="Momentum" score={current.score} />
              </div>
              {current.tasks.length > 0 && (
                <div style={{ fontSize: "12px", color: "rgba(255,255,255,0.4)" }}>
                  {current.tasks.length} pending task{current.tasks.length > 1 ? "s" : ""}
                </div>
              )}
            </div>
          ) : (
            <div style={{ fontSize: "13px", color: "rgba(255,255,255,0.35)", padding: "16px" }}>
              No active projects with tasks found. Start a voice session to capture your work.
            </div>
          )}
        </div>

        {/* Panel 2: Fog Check */}
        <div style={{ marginBottom: "28px" }}>
          <Label>Yesterday's Fog</Label>
          <p style={{ fontSize: "13px", color: "rgba(255,255,255,0.65)", margin: 0, lineHeight: 1.6 }}>
            {fogLines}
          </p>
        </div>

        {/* Panel 3: Open Tasks */}
        <div style={{ marginBottom: "32px" }}>
          <Label>
            {current ? `Tasks for ${current.node.label}` : "Pending Tasks"}
          </Label>
          {(current ? current.tasks : pendingTasks.slice(0, 3)).length > 0 ? (
            <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
              {(current ? current.tasks : pendingTasks.slice(0, 3)).map((t) => (
                <div
                  key={t.id}
                  style={{
                    padding: "10px 14px",
                    background: "rgba(255,255,255,0.04)",
                    borderRadius: "8px",
                    borderLeft: "2px solid #38bdf8",
                    fontSize: "13px",
                    color: "rgba(255,255,255,0.85)",
                  }}
                >
                  {t.title}
                </div>
              ))}
            </div>
          ) : (
            <div style={{ fontSize: "13px", color: "rgba(255,255,255,0.35)" }}>
              No tasks extracted yet.{current ? ` Start a note about ${current.node.label} to capture some.` : ""}
            </div>
          )}
        </div>

        {/* Start Focus */}
        <button
          onClick={() => {
            if (!current) return;
            localStorage.setItem("nodemind_focus_project", current.node.label);
            onStartFocus?.(current.node.label);
            setStarted(true);
          }}
          disabled={!current}
          style={{
            width: "100%",
            padding: "14px",
            borderRadius: "10px",
            border: "none",
            background: current ? "#6366f1" : "rgba(255,255,255,0.08)",
            color: current ? "#fff" : "rgba(255,255,255,0.3)",
            fontSize: "15px",
            fontWeight: 700,
            cursor: current ? "pointer" : "not-allowed",
          }}
        >
          Start Focus
        </button>

      </div>
    </div>
  );
};

const overlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(9,9,11,0.92)",
  backdropFilter: "blur(8px)",
  zIndex: 200,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const innerStyle: React.CSSProperties = {
  width: "480px",
  maxWidth: "calc(100vw - 32px)",
  maxHeight: "90vh",
  overflowY: "auto",
  background: "#111827",
  borderRadius: "16px",
  border: "1px solid rgba(255,255,255,0.1)",
  padding: "24px",
  boxShadow: "0 24px 80px rgba(0,0,0,0.6)",
};

const arrowBtn: React.CSSProperties = {
  width: "28px",
  height: "28px",
  borderRadius: "6px",
  border: "1px solid rgba(255,255,255,0.15)",
  background: "transparent",
  color: "rgba(255,255,255,0.6)",
  cursor: "pointer",
  fontSize: "14px",
};

function Label({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontSize: "11px",
      color: "rgba(255,255,255,0.35)",
      textTransform: "uppercase",
      letterSpacing: "0.08em",
      fontWeight: 600,
      marginBottom: "10px",
    }}>
      {children}
    </div>
  );
}

function btnStyle(bg: string, color: string): React.CSSProperties {
  return {
    padding: "10px 24px",
    borderRadius: "8px",
    border: "1px solid rgba(255,255,255,0.2)",
    background: bg,
    color,
    fontSize: "14px",
    cursor: "pointer",
    fontWeight: 600,
  };
}

export default FocusMode;
