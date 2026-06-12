import React from "react";
import type { GraphLens } from "../types";

interface GraphLensBarProps {
  activeLens: GraphLens;
  onChange: (lens: GraphLens) => void;
}

const LENSES: { id: GraphLens; label: string }[] = [
  { id: "memory", label: "Memory" },
  { id: "tasks", label: "Tasks" },
  { id: "goals", label: "Goals" },
  { id: "week", label: "Week" },
];

const GraphLensBar: React.FC<GraphLensBarProps> = ({ activeLens, onChange }) => {
  return (
    <div style={{
      display: "flex",
      gap: "8px",
      padding: "8px 16px",
      background: "rgba(0,0,0,0.3)",
      borderBottom: "1px solid rgba(255,255,255,0.08)",
    }}>
      {LENSES.map((lens) => (
        <button
          key={lens.id}
          onClick={() => onChange(lens.id)}
          style={{
            padding: "4px 14px",
            borderRadius: "12px",
            border: activeLens === lens.id ? "1px solid #6366f1" : "1px solid rgba(255,255,255,0.15)",
            background: activeLens === lens.id ? "#6366f1" : "transparent",
            color: activeLens === lens.id ? "#fff" : "rgba(255,255,255,0.6)",
            fontSize: "13px",
            cursor: "pointer",
            fontWeight: activeLens === lens.id ? 600 : 400,
            transition: "all 0.15s ease",
          }}
        >
          {lens.label}
        </button>
      ))}
    </div>
  );
};

export default GraphLensBar;
