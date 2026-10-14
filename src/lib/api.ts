// Thin typed wrappers over the Tauri IPC surface.
// Keep 1:1 with the commands registered in `src-tauri/src/lib.rs`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CalendarSync,
  CaptureMode,
  CaptureResult,
  CaptureStatus,
  EngineStatus,
  GraphData,
  ModelStatusView,
  Settings,
  Thought,
  SearchHit,
  TaskRef,
  Today,
  UpdateInfo,
} from "./types";

// ── engine ──────────────────────────────────────────────────────────────────

export const engineStatus = () => invoke<EngineStatus>("engine_status");
export const beginSetup = () => invoke<void>("begin_setup");

// ── capture ─────────────────────────────────────────────────────────────────

// The backend owns the microphone; these only drive the session. Progress
// streams over the `capture-*` events (see lib/useCapture.ts).
export const startCapture = (language?: string, mode: CaptureMode = "note") =>
  invoke<void>("start_capture", { language: language ?? null, mode });

export const stopCapture = () => invoke<CaptureResult>("stop_capture");

export const cancelCapture = () => invoke<void>("cancel_capture");

export const captureStatus = () => invoke<CaptureStatus>("capture_status");

/** Open the full window (from the menu-bar popover), optionally on a view. */
export const openMainWindow = (view?: string) =>
  invoke<void>("open_main_window", { view: view ?? null });

// ── thoughts / tasks / graph ────────────────────────────────────────────────

export const getThought = (id: string) =>
  invoke<Thought | null>("get_thought", { id });

export const deleteThought = (id: string) =>
  invoke<void>("delete_thought", { id });

/** Add a typed task; resolves to null when it duplicates an open task. */
export const addTask = (title: string) => invoke<TaskRef | null>("add_task", { title });

/** The Today screen: open checklist (with sources) + today's captures. */
export const getToday = () => invoke<Today>("get_today");

export const updateTask = (id: string, title?: string | null, done?: boolean | null) =>
  invoke<void>("update_task", { id, title: title ?? null, done: done ?? null });

export const deleteTask = (id: string) => invoke<void>("delete_task", { id });

export const getGraph = (limit?: number) =>
  invoke<GraphData>("get_graph", { limit: limit ?? null });

/** Semantic search (EmbeddingGemma 2) with a keyword boost, over every transcript. */
export const searchMemory = (query: string, k?: number) =>
  invoke<SearchHit[]>("search_memory", { query, k: k ?? null });

// ── calendar / today / focus ────────────────────────────────────────────────

export const syncCalendar = () => invoke<CalendarSync>("sync_calendar");


// ── settings / updates ──────────────────────────────────────────────────────

export const getSettings = () => invoke<Settings>("get_settings");
export const setLanguage = (language: string) =>
  invoke<void>("set_language", { language });
export const openDataDir = () => invoke<void>("open_data_dir");
export const reindexEmbeddings = () => invoke<number>("reindex_embeddings");

export const checkForUpdates = () => invoke<UpdateInfo | null>("check_for_updates");
export const installUpdate = () => invoke<void>("install_update");

// ── diagnostics ─────────────────────────────────────────────────────────────

/** Append a UI error to app-data/logs/ui.log (never throws). */
export function logUiError(message: string): void {
  const where = typeof window !== "undefined" ? window.location.hash || "/" : "";
  void invoke("log_ui_error", { message: `[${where}] ${message}` }).catch(() => undefined);
}

// ── events ──────────────────────────────────────────────────────────────────

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload));
}

// Model progress events stream from the downloader; merge into a status view.
export function applyModelProgress(
  status: EngineStatus,
  p: { key: string; done: number; total: number },
): EngineStatus {
  const { done, total } = p;
  return {
    ...status,
    models: status.models.map((m: ModelStatusView) =>
      m.key === p.key
        ? {
            ...m,
            downloading: true,
            expectedBytes: total > 0 ? total : m.expectedBytes,
            downloadedBytes: done,
          }
        : m,
    ),
  };
}
