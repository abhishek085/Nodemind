import React from "react";
import type { DriftAlert } from "../types";

interface DriftAlertCardProps {
  alert: DriftAlert;
  onKeep: (id: string) => void;
  onArchive: (id: string) => void;
  onSnooze: (id: string) => void;
}

const DriftAlertCard: React.FC<DriftAlertCardProps> = ({ alert, onKeep, onArchive, onSnooze }) => {
  const intensity = Math.round(alert.drift_score * 100);
  const borderColor = alert.drift_score > 0.7 ? "#f87171" : alert.drift_score > 0.4 ? "#fbbf24" : "#6b7280";

  return (
    <div style={{
      padding: "10px 14px",
      borderRadius: "8px",
      border: `1px solid ${borderColor}`,
      background: "rgba(0,0,0,0.25)",
      display: "flex",
      alignItems: "center",
      gap: "12px",
      flexShrink: 0,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: "12px", color: "rgba(255,255,255,0.5)", marginBottom: "2px" }}>
          Drifting goal · {alert.days_since_mentioned}d ago · {intensity}%
        </div>
        <div style={{ fontSize: "14px", color: "#fff", fontWeight: 500, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {alert.goal}
        </div>
      </div>
      <div style={{ display: "flex", gap: "6px", flexShrink: 0 }}>
        <button onClick={() => onKeep(alert.goal_id)} style={btnStyle("#34d399")}>Keep</button>
        <button onClick={() => onSnooze(alert.goal_id)} style={btnStyle("#fbbf24")}>Snooze</button>
        <button onClick={() => onArchive(alert.goal_id)} style={btnStyle("#6b7280")}>Archive</button>
      </div>
    </div>
  );
};

function btnStyle(color: string): React.CSSProperties {
  return {
    padding: "3px 10px",
    borderRadius: "6px",
    border: `1px solid ${color}`,
    background: "transparent",
    color: color,
    fontSize: "12px",
    cursor: "pointer",
    fontWeight: 500,
  };
}

export default DriftAlertCard;
