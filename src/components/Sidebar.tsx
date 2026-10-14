// Icon rail on the left edge — the whole app is four destinations.

import type { ReactNode } from "react";

export type ViewId = "capture" | "graph" | "today" | "settings";

interface Props {
  view: ViewId;
  onNavigate: (v: ViewId) => void;
}

const ITEMS: { id: ViewId; label: string; icon: ReactNode }[] = [
  {
    id: "capture",
    label: "Capture",
    icon: (
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0M12 18v3" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: "graph",
    label: "Mind Map",
    icon: (
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8">
        <circle cx="12" cy="5" r="2.2" />
        <circle cx="5" cy="18" r="2.2" />
        <circle cx="19" cy="18" r="2.2" />
        <path d="M10.8 6.8 6 16M13.2 6.8 18 16M7.2 18h9.6" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: "today",
    label: "Today",
    icon: (
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8">
        <circle cx="12" cy="12" r="8.5" />
        <path d="M12 7.5V12l3 2" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    icon: (
      <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8">
        <circle cx="12" cy="12" r="3" />
        <path
          d="M19 12a7 7 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 0 0-2-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7 7 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7 7 0 0 0 2 1.2L10 21h4l.5-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.07-.4.1-.8.1-1.2Z"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
];

export default function Sidebar({ view, onNavigate }: Props) {
  return (
    <nav className="no-drag flex w-16 shrink-0 flex-col items-center gap-2 border-r border-line bg-surface-2 py-4">
      <button
        onClick={() => onNavigate("capture")}
        className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-gradient-to-br from-accent to-accent-2 text-white shadow-sm"
        title="Nodemind"
      >
        <svg viewBox="0 0 24 24" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="7" cy="7" r="2.5" />
          <circle cx="17" cy="9" r="2.5" />
          <circle cx="11" cy="17" r="2.5" />
          <path d="M8.8 8.3 14.7 9M8 9.2l2.2 5.4M15.5 11l-3 4.4" strokeWidth="1.5" />
        </svg>
      </button>
      {ITEMS.map((it) => (
        <button
          key={it.id}
          onClick={() => onNavigate(it.id)}
          title={it.label}
          className={`flex h-11 w-11 flex-col items-center justify-center gap-0.5 rounded-xl transition-colors ${
            view === it.id
              ? "bg-surface text-accent shadow-sm"
              : "text-ink-3 hover:bg-surface-2 hover:text-ink-2"
          }`}
        >
          {it.icon}
          <span className="text-[9px] font-medium leading-none">{it.label.split(" ")[0]}</span>
        </button>
      ))}
    </nav>
  );
}
