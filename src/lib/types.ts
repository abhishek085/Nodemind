// Domain types mirroring the Rust backend's serde shapes (camelCase).

// ── Engine ──────────────────────────────────────────────────────────────────

export type EnginePhase = "setup" | "starting" | "ready" | "failed";

export interface ModelStatusView {
  key: string; // "whisper" | "llm" | "embed"
  label: string;
  present: boolean;
  path: string | null;
  sizeBytes: number;
  expectedBytes: number;
  downloading: boolean;
  downloadedBytes: number;
  error: string | null;
}

export interface EngineStatus {
  phase: EnginePhase;
  arch: string;
  chatRunning: boolean;
  embedRunning: boolean;
  coreml: boolean;
  models: ModelStatusView[];
  error: string | null;
}

export interface ModelProgress {
  key: string;
  done: number;
  total: number;
}

// ── Capture ─────────────────────────────────────────────────────────────────

export interface CaptureSession {
  language: string;
  maxSeconds: number;
  startedAt: number;
}

export interface CommandIntent {
  intent: "create_task" | "start_focus" | "unknown";
  title: string | null;
  minutes: number | null;
}

export interface CaptureResult {
  thoughtId: string;
  transcript: string;
  summary: string | null;
  tasks: string[];
  topics: string[];
  people: string[];
  related: string[];
  command: CommandIntent | null;
  durationSecs: number;
}

export type CaptureStage = "transcribing" | "organizing" | "done";

// ── Thoughts / tasks / graph ────────────────────────────────────────────────

export interface Thought {
  id: string;
  text: string;
  summary: string | null;
  language: string;
  day: string; // YYYY-MM-DD (local)
  startedAt: string; // RFC3339
  durationSecs: number;
  createdAt: string;
}

export interface Task {
  id: string;
  title: string;
  project: string | null;
  thoughtId: string | null;
  done: boolean;
  createdAt: string;
}

export type NodeType =
  | "thought"
  | "topic"
  | "person"
  | "goal"
  | "task"
  | "fog";

export interface GraphEdge {
  id: string;
  fromNode: string;
  toNode: string;
  edgeType: string;
  weight: number;
}

export interface NodeView {
  id: string;
  nodeType: NodeType;
  label: string;
  mentions: number;
  sentiment: string; // positive | neutral | negative
  excerpt: string | null;
  summary: string | null;
  lastSeen: string | null;
  degree: number;
}

export interface GraphData {
  nodes: NodeView[];
  edges: GraphEdge[];
}

export interface ThoughtHit {
  thought: Thought;
  score: number;
}

// ── Calendar / today / focus ────────────────────────────────────────────────

export interface CalendarEvent {
  id: string;
  title: string;
  start: string; // RFC3339 local
  end: string;
  calendar: string;
  day: string;
}

export interface CalendarSync {
  day: string;
  events: CalendarEvent[];
  scheduledHours: number;
  permissionHint: string | null;
}

export interface TodayBrief {
  friction: string | null;
  suggestion: string | null;
  openTasks: number;
  thoughtsToday: number;
  eventsToday: number;
  scheduledHours: number;
  focusMinutes: number;
}

export interface FocusSession {
  id: string;
  title: string | null;
  day: string;
  startedAt: string;
  endedAt: string | null;
  minutes: number;
}

export interface FocusStarted {
  id: string;
  startedAt: string;
  minutes: number;
}

// ── Settings / updates ──────────────────────────────────────────────────────

export interface Settings {
  version: string;
  arch: string;
  language: string;
  dataDir: string;
  modelsDir: string;
  promptsPath: string | null;
  counts: {
    thoughts: number;
    tasksOpen: number;
    vectors: number;
    nodes: number;
  };
}

export interface UpdateInfo {
  version: string;
  notes: string | null;
}
