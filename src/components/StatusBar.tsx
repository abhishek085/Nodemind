// Slim strip under the title bar: engine health at a glance.

import type { EngineStatus } from "../lib/types";

interface Props {
  status: EngineStatus | null;
  viewTitle: string;
}

const PHASE_META: Record<string, { label: string; dot: string }> = {
  setup: { label: "Setting up…", dot: "bg-warn" },
  starting: { label: "Waking the engine…", dot: "bg-warn animate-pulse" },
  ready: { label: "Local engine ready", dot: "bg-ok" },
  failed: { label: "Engine error", dot: "bg-danger" },
};

export default function StatusBar({ status, viewTitle }: Props) {
  const phase = status?.phase ?? "setup";
  const meta = PHASE_META[phase];

  return (
    <header className="drag-region flex h-9 shrink-0 items-center gap-2 border-b border-line bg-surface-2 pl-[88px] pr-4">
      <div className="no-drag flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${meta.dot}`} />
        <span className="text-xs font-medium text-ink-2">{meta.label}</span>
      </div>
      <span className="no-drag ml-auto text-xs font-semibold tracking-wide text-ink-3">
        {viewTitle}
      </span>
    </header>
  );
}
