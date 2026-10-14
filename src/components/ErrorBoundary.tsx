// Last line of defence: a render error shows a recoverable card (and is
// logged to app-data/logs/ui.log) instead of unmounting to a blank window.

import { Component, type ReactNode } from "react";
import { logUiError } from "../lib/api";

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    logUiError(`${error.message}\n${error.stack ?? ""}\n${info.componentStack ?? ""}`);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 bg-bg p-6 text-center text-ink">
        <p className="text-sm font-semibold">Something went wrong on this screen</p>
        <p className="max-w-xs break-words font-mono text-[11px] text-ink-3">{this.state.error.message}</p>
        <button
          onClick={() => window.location.reload()}
          className="rounded-lg bg-accent px-4 py-1.5 text-xs font-semibold text-white"
        >
          Reload
        </button>
      </div>
    );
  }
}
