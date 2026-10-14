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
  models: ModelStatusView[];
  error: string | null;
}

export interface ModelProgress {
  key: string;
  done: number;
  total: number;
}

// ── Capture ─────────────────────────────────────────────────────────────────

/** "note" = brain-dump (mic); "meeting" = mic ("You") + call audio ("Them"). */
export type CaptureMode = "note" | "meeting";

export interface CaptureStatus {
  recording: boolean;
  mode: CaptureMode | null;
  elapsedSecs: number;
  partial: string;
}

/** A task a capture added to Today (undo = delete it). */
export interface TaskRef {
  id: string;
  title: string;
}

export interface CommandIntent {
  intent: "create_task" | "unknown";
  title: string | null;
}

export interface CaptureResult {
  thoughtId: string;
  transcript: string;
  summary: string | null;
  tasks: TaskRef[];
  topics: string[];
  people: string[];
  related: string[];
  command: CommandIntent | null;
  durationSecs: number;
  mode: CaptureMode;
  /** Meeting mode only. */
  minutes?: MeetingMinutes;
  /** Where the thinking seemed stuck; empty for clear speech. */
  fog: FogSignal[];
}

export interface ActionItem {
  owner: string; // "You", "Them", or a name
  task: string;
  due: string;
}

export interface MeetingMinutes {
  title: string;
  summary: string;
  decisions: string[];
  actionItems: ActionItem[];
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
  kind: "note" | "meeting";
  /** JSON of MeetingMinutes (meetings only). */
  minutes: string | null;
  /** JSON of FogSignal[] (when any were found). */
  fog: string | null;
}

/** Where the thinking seemed stuck (engine/fog.rs). */
export interface FogSignal {
  tag: "overthinking" | "unclarity" | "abstraction" | "overwhelmed" | string;
  context: string;
  question: string;
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

/** Best-matching passage of one thought (see engine/search.rs). */
export interface SearchHit {
  thoughtId: string;
  snippet: string;
  summary: string | null;
  day: string; // YYYY-MM-DD (local)
  startedAt: string;
  score: number;
  /** Clearly above the rest of your history, not just the closest. */
  strong: boolean;
  /** Nothing clearly matched; these are just the closest. */
  weak: boolean;
  /** Query words found verbatim in the snippet. */
  matched: string[];
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

/** The Today screen in one payload (see commands/thoughts.rs `get_today`). */
export interface Today {
  day: string;
  tasks: TodayTask[];
  thoughts: TodayThought[];
}

export interface TodayTask {
  id: string;
  title: string;
  createdAt: string;
  /** The capture it came from; typed tasks have none. */
  source: { thoughtId: string; kind: "note" | "meeting"; title: string; startedAt: string } | null;
}

export interface TodayThought {
  id: string;
  kind: "note" | "meeting";
  startedAt: string;
  durationSecs: number;
  title: string;
  text: string;
  minutes: MeetingMinutes | null;
  fog: FogSignal[] | null;
  taskCount: number;
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
