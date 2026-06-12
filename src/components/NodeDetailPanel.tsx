import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GraphNode, GraphEdge } from "../types";

interface NodeDetailPanelProps {
  node: GraphNode | null;
  allNodes: GraphNode[];
  allEdges: GraphEdge[];
  momentumScores: Record<string, number>;
  onClose: () => void;
  onNavigateToNode: (id: string) => void;
  onAddNote?: (entityLabel: string) => void;
}

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

const NodeDetailPanel: React.FC<NodeDetailPanelProps> = ({
  node,
  allNodes,
  allEdges,
  momentumScores,
  onClose,
  onNavigateToNode,
  onAddNote,
}) => {
  const [recentNotes, setRecentNotes] = useState<string[]>([]);
  const [actionMsg, setActionMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!node) return;
    setRecentNotes([]);
    invoke<string[]>("get_node_related_notes", { nodeId: node.id })
      .then(setRecentNotes)
      .catch(() => setRecentNotes([]));
  }, [node?.id]);

  if (!node) return null;

  const typeColor = NODE_TYPE_COLORS[node.node_type] ?? "#9ca3af";
  const momentum = momentumScores[node.id] ?? 0;
  const momentumPct = Math.round(momentum * 100);

  const connectedEdges = allEdges.filter(
    (e) => e.from_node === node.id || e.to_node === node.id
  );
  const connectedIds = new Set(
    connectedEdges.flatMap((e) => [e.from_node, e.to_node]).filter((id) => id !== node.id)
  );
  const connectedNodes = allNodes.filter((n) => connectedIds.has(n.id));

  // Check if any connected node is a fog signal
  const hasFogEdge = connectedNodes.some(
    (n) => n.node_type === "fog" || n.node_type === "fog_pattern"
  );

  const handleArchive = async () => {
    try {
      await invoke("archive_node", { nodeId: node.id });
      setActionMsg("Archived");
      setTimeout(onClose, 800);
    } catch {
      setActionMsg("Error archiving");
    }
  };

  const handleMarkAsGoal = async () => {
    try {
      await invoke("mark_node_as_goal", { nodeId: node.id });
      setActionMsg("Marked as goal");
      setTimeout(onClose, 800);
    } catch {
      setActionMsg("Error updating");
    }
  };

  const lastMentionedDaysAgo = node.last_mentioned_at
    ? Math.floor(
        (Date.now() - new Date(node.last_mentioned_at).getTime()) / 86400000
      )
    : null;

  return (
    <div
      style={{
        position: "fixed",
        right: 0,
        top: 0,
        bottom: 0,
        width: "320px",
        background: "#111827",
        borderLeft: "1px solid rgba(255,255,255,0.1)",
        display: "flex",
        flexDirection: "column",
        zIndex: 100,
        boxShadow: "-4px 0 24px rgba(0,0,0,0.4)",
        animation: "slideIn 0.2s ease",
      }}
    >
      {/* Header */}
      <div style={{ padding: "16px", borderBottom: "1px solid rgba(255,255,255,0.08)" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <span
              style={{
                fontSize: "11px",
                color: typeColor,
                textTransform: "uppercase",
                letterSpacing: "0.08em",
                fontWeight: 600,
              }}
            >
              {node.node_type}
            </span>
            <div
              style={{
                fontSize: "18px",
                fontWeight: 700,
                color: "#fff",
                marginTop: "4px",
                wordBreak: "break-word",
              }}
            >
              {node.label}
            </div>
          </div>
          <button
            onClick={onClose}
            style={{
              background: "none",
              border: "none",
              color: "rgba(255,255,255,0.5)",
              fontSize: "18px",
              cursor: "pointer",
              padding: "0 0 0 8px",
              flexShrink: 0,
            }}
          >
            ✕
          </button>
        </div>

        {/* Stats row */}
        <div style={{ display: "flex", gap: "16px", marginTop: "10px" }}>
          <StatBadge label="Mentions" value={String(node.mentions_count)} />
          {lastMentionedDaysAgo !== null && (
            <StatBadge
              label="Last seen"
              value={lastMentionedDaysAgo === 0 ? "Today" : `${lastMentionedDaysAgo}d ago`}
            />
          )}
          {momentumPct > 0 && <StatBadge label="Momentum" value={`${momentumPct}%`} color="#34d399" />}
        </div>
      </div>

      {/* Momentum bar */}
      {momentum > 0 && (
        <div style={{ padding: "10px 16px", borderBottom: "1px solid rgba(255,255,255,0.06)" }}>
          <div style={{ height: "3px", background: "rgba(255,255,255,0.1)", borderRadius: "2px" }}>
            <div
              style={{
                height: "100%",
                width: `${momentumPct}%`,
                background: momentum > 0.7 ? "#34d399" : momentum > 0.4 ? "#fbbf24" : "#f87171",
                borderRadius: "2px",
                transition: "width 0.5s ease",
              }}
            />
          </div>
        </div>
      )}

      {/* Fog signal indicator */}
        {hasFogEdge && (
          <div style={{
            padding: "8px 16px",
            background: "rgba(248,113,113,0.08)",
            borderLeft: "3px solid #f87171",
            marginBottom: "0",
            borderBottom: "1px solid rgba(255,255,255,0.06)",
          }}>
            <span style={{ fontSize: "11px", color: "#f87171", fontWeight: 600 }}>⚠ Fog signal detected near this entity</span>
          </div>
        )}

      {/* Scrollable body */}
      <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px" }}>
        {/* Connected entities */}
        {connectedNodes.length > 0 && (
          <section style={{ marginBottom: "16px" }}>
            <SectionTitle>Connected ({connectedNodes.length})</SectionTitle>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "6px", marginTop: "8px" }}>
              {connectedNodes.map((n) => (
                <button
                  key={n.id}
                  onClick={() => onNavigateToNode(n.id)}
                  style={{
                    padding: "3px 10px",
                    borderRadius: "12px",
                    border: `1px solid ${NODE_TYPE_COLORS[n.node_type] ?? "#6b7280"}`,
                    background: "transparent",
                    color: NODE_TYPE_COLORS[n.node_type] ?? "#9ca3af",
                    fontSize: "12px",
                    cursor: "pointer",
                  }}
                >
                  {n.label}
                </button>
              ))}
            </div>
          </section>
        )}

        {/* Recent notes */}
        {recentNotes.length > 0 && (
          <section style={{ marginBottom: "16px" }}>
            <SectionTitle>Recent Mentions</SectionTitle>
            <div style={{ marginTop: "8px", display: "flex", flexDirection: "column", gap: "8px" }}>
              {recentNotes.slice(0, 5).map((note, i) => (
                <div
                  key={i}
                  style={{
                    fontSize: "12px",
                    color: "rgba(255,255,255,0.65)",
                    padding: "8px",
                    background: "rgba(255,255,255,0.04)",
                    borderRadius: "6px",
                    lineHeight: 1.5,
                  }}
                >
                  {note}
                </div>
              ))}
            </div>
          </section>
        )}

        {recentNotes.length === 0 && connectedNodes.length === 0 && (
          <div style={{ fontSize: "13px", color: "rgba(255,255,255,0.35)", textAlign: "center", paddingTop: "32px" }}>
            No related notes found yet.
          </div>
        )}
      </div>

      {/* Quick actions */}
      <div
        style={{
          padding: "12px 16px",
          borderTop: "1px solid rgba(255,255,255,0.08)",
          display: "flex",
          flexDirection: "column",
          gap: "8px",
        }}
      >
        {actionMsg ? (
          <div style={{ fontSize: "13px", color: "#34d399", textAlign: "center", padding: "6px" }}>{actionMsg}</div>
        ) : (
          <div style={{ display: "flex", gap: "8px" }}>
            <ActionBtn color="#34d399" onClick={handleMarkAsGoal}>Mark as goal</ActionBtn>
            <ActionBtn color="#f87171" onClick={handleArchive}>Archive</ActionBtn>
          </div>
        )}
        {onAddNote && (
          <ActionBtn color="#38bdf8" onClick={() => { onAddNote(node.label); onClose(); }}>
            + Add note
          </ActionBtn>
        )}
      </div>

      <style>{`
        @keyframes slideIn {
          from { transform: translateX(100%); }
          to { transform: translateX(0); }
        }
      `}</style>
    </div>
  );
};

function StatBadge({ label, value, color }: { label: string; value: string; color?: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "1px" }}>
      <span style={{ fontSize: "10px", color: "rgba(255,255,255,0.4)", textTransform: "uppercase" }}>{label}</span>
      <span style={{ fontSize: "13px", color: color ?? "rgba(255,255,255,0.85)", fontWeight: 500 }}>{value}</span>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: "11px", color: "rgba(255,255,255,0.4)", textTransform: "uppercase", letterSpacing: "0.06em", fontWeight: 600 }}>
      {children}
    </div>
  );
}

function ActionBtn({ children, color, onClick }: { children: React.ReactNode; color: string; onClick?: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{
        flex: 1,
        padding: "6px 0",
        borderRadius: "6px",
        border: `1px solid ${color}`,
        background: "transparent",
        color,
        fontSize: "12px",
        cursor: "pointer",
        fontWeight: 500,
      }}
    >
      {children}
    </button>
  );
}

export default NodeDetailPanel;
