import React from "react";
import type { View } from "../types";

interface SidebarProps {
  activeView: View;
  onNav: (view: View) => void;
  onFocus: () => void;
  activeFocusProject?: string | null;
}

const PRIMARY_NAV: { id: View; label: string; emoji: string }[] = [
  { id: "dashboard", label: "Dashboard",       emoji: "🏠" },
  { id: "graph",     label: "Knowledge Graph", emoji: "🧠" },
];

const Sidebar: React.FC<SidebarProps> = ({ activeView, onNav, onFocus, activeFocusProject }) => {
  return (
    <nav className="sidebar">
      {PRIMARY_NAV.map((item) => (
        <button
          key={item.id}
          className={`nav-btn${activeView === item.id ? " nav-active" : ""}`}
          onClick={() => onNav(item.id)}
        >
          <span className="nav-emoji">{item.emoji}</span>
          <span className="nav-label">{item.label}</span>
        </button>
      ))}

      {/* Focus Mode — triggers overlay, not a route */}
      <button
        className="nav-btn"
        onClick={onFocus}
        title="Open Focus Mode"
      >
        <span className="nav-emoji">🎯</span>
        <span className="nav-label">Focus</span>
      </button>

      <div className="sidebar-spacer" />

      {/* Active focus project indicator */}
      {activeFocusProject && (
        <div
          style={{
            fontSize: "9px",
            color: "#6366f1",
            textAlign: "center",
            padding: "4px 6px",
            background: "rgba(99,102,241,0.12)",
            borderRadius: "6px",
            marginBottom: "4px",
            lineHeight: 1.4,
            wordBreak: "break-word",
          }}
          title={`Focused on: ${activeFocusProject}`}
        >
          Focus: {activeFocusProject}
        </div>
      )}

      {/* Settings — small icon at bottom */}
      <button
        className={`nav-btn${activeView === "settings" ? " nav-active" : ""}`}
        onClick={() => onNav("settings")}
        title="Settings"
      >
        <span className="nav-emoji">⚙️</span>
        <span className="nav-label">Settings</span>
      </button>

      <div className="sidebar-brand">
        <img src="/nodemind_icon.svg" alt="Nodemind" className="nokast-logo" />
      </div>
    </nav>
  );
};

export default Sidebar;
