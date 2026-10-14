// Non-blocking strip for app updates (Tauri updater → relaunch).

import { useState } from "react";
import { installUpdate } from "../lib/api";
import type { UpdateInfo } from "../lib/types";

interface Props {
  update: UpdateInfo | null;
  onDismiss: () => void;
}

export default function UpdateBanner({ update, onDismiss }: Props) {
  const [installing, setInstalling] = useState(false);

  if (!update) return null;

  const install = async () => {
    setInstalling(true);
    try {
      await installUpdate(); // backend relaunches the app; UI may vanish mid-await
    } catch (e) {
      alert(`Could not install the update: ${String(e)}`);
      setInstalling(false);
    }
  };

  return (
    <div className="no-drag flex shrink-0 items-center gap-3 border-b border-line bg-accent/10 px-4 py-2 fade-up">
      <span className="text-xs font-semibold text-ink">
        Nodemind {update.version} is available
      </span>
      {update.notes && <span className="truncate text-[11px] text-ink-3">{update.notes}</span>}
      <button
        onClick={install}
        disabled={installing}
        className="ml-auto rounded-lg bg-accent px-3 py-1 text-xs font-semibold text-white hover:opacity-90 disabled:opacity-50"
      >
        {installing ? "Installing…" : "Restart to update"}
      </button>
      <button
        onClick={onDismiss}
        className="rounded-lg px-2 py-1 text-xs text-ink-3 hover:text-ink"
        title="Not now"
      >
        ✕
      </button>
    </div>
  );
}
