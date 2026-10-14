// Thin typed wrappers over the Tauri IPC surface.
// Keep 1:1 with the commands registered in `src-tauri/src/lib.rs`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CalendarSync,
  CaptureResult,
  CaptureSession,
  EngineStatus,
  FocusSession,
  FocusStarted,
  GraphData,
  ModelStatusView,
  Settings,
  Task,
  Thought,
  ThoughtHit,
  TodayBrief,
  UpdateInfo,
} from "./types";

// ── engine ──────────────────────────────────────────────────────────────────

export const engineStatus = () => invoke<EngineStatus>("engine_status");
export const beginSetup = () => invoke<void>("begin_setup");
export const restartEngine = () => invoke<void>("restart_engine");

// ── capture ─────────────────────────────────────────────────────────────────

export const beginCapture = (language?: string) =>
  invoke<CaptureSession>("begin_capture", { language: language ?? null });

export const appendAudio = (chunkBase64: string) =>
  invoke<number>("append_audio", { chunk: chunkBase64 });

export const finishCapture = () => invoke<CaptureResult>("finish_capture");

// ── thoughts / tasks / graph ────────────────────────────────────────────────

export const getThoughts = (limit?: number) =>
  invoke<Thought[]>("get_thoughts", { limit: limit ?? null });

export const getThought = (id: string) =>
  invoke<Thought | null>("get_thought", { id });

export const deleteThought = (id: string) =>
  invoke<void>("delete_thought", { id });

export const getTasks = (includeDone?: boolean) =>
  invoke<Task[]>("get_tasks", { includeDone: includeDone ?? null });

export const addTask = (title: string, project?: string | null) =>
  invoke<Task>("add_task", { title, project: project ?? null });

export const updateTask = (id: string, title?: string | null, done?: boolean | null) =>
  invoke<void>("update_task", { id, title: title ?? null, done: done ?? null });

export const deleteTask = (id: string) => invoke<void>("delete_task", { id });

export const getGraph = (limit?: number) =>
  invoke<GraphData>("get_graph", { limit: limit ?? null });

export const searchThoughts = (query: string, k?: number) =>
  invoke<ThoughtHit[]>("search_thoughts", { query, k: k ?? null });

// ── calendar / today / focus ────────────────────────────────────────────────

export const syncCalendar = () => invoke<CalendarSync>("sync_calendar");

export const getTodayBrief = () => invoke<TodayBrief>("get_today_brief");

export const startFocus = (minutes?: number) =>
  invoke<FocusStarted>("start_focus", { minutes: minutes ?? null });

export const finishFocus = (id: string, minutes?: number | null) =>
  invoke<void>("finish_focus", { id, minutes: minutes ?? null });

export const getFocusToday = () => invoke<FocusSession[]>("get_focus_today");

// ── settings / updates ──────────────────────────────────────────────────────

export const getSettings = () => invoke<Settings>("get_settings");
export const setLanguage = (language: string) =>
  invoke<void>("set_language", { language });
export const openDataDir = () => invoke<void>("open_data_dir");
export const reindexEmbeddings = () => invoke<number>("reindex_embeddings");

export const checkForUpdates = () => invoke<UpdateInfo | null>("check_for_updates");
export const installUpdate = () => invoke<void>("install_update");

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
