import React from "react";

interface MomentumBarProps {
  label: string;
  score: number; // 0.0–1.0
  rank?: number; // optional rank label
}

const MomentumBar: React.FC<MomentumBarProps> = ({ label, score, rank }) => {
  const pct = Math.round(score * 100);
  const color = score > 0.7 ? "#34d399" : score > 0.4 ? "#fbbf24" : "#f87171";

  return (
    <div style={{ marginBottom: "10px" }}>
      <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "4px" }}>
        <span style={{ fontSize: "13px", color: "rgba(255,255,255,0.85)", display: "flex", alignItems: "center", gap: "6px" }}>
          {rank !== undefined && (
            <span style={{ fontSize: "11px", color: "rgba(255,255,255,0.4)", minWidth: "16px" }}>#{rank}</span>
          )}
          {label}
        </span>
        <span style={{ fontSize: "12px", color, fontWeight: 600 }}>{pct}%</span>
      </div>
      <div style={{ height: "4px", borderRadius: "2px", background: "rgba(255,255,255,0.1)", overflow: "hidden" }}>
        <div
          style={{
            height: "100%",
            width: `${pct}%`,
            background: color,
            borderRadius: "2px",
            transition: "width 0.6s ease",
          }}
        />
      </div>
    </div>
  );
};

export default MomentumBar;
