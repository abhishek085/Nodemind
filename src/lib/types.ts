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

export interface CaptureResult {
  thoughtId: string;
  transcript: string;
  summary: string | null;
  /** Tasks, already added to Today. */
  tasks: TaskRef[];
  /** Ideas, learnings, questions, decisions. */
  items: ItemRef[];
  topics: string[];
  people: string[];
  /** Nothing worth keeping was said (small talk, a test). */
  noise: boolean;
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

/** Mind-map node types: items said (engine/sort.rs), what they're about, fog. */
export type NodeType = "idea" | "learning" | "question" | "decision" | "topic" | "person" | "goal" | "fog";

/** An idea / learning / question / decision kept from a recording. */
export interface ItemRef {
  id: string;
  kind: "idea" | "learning" | "question" | "decision";
  text: string;
}

export interface Item extends ItemRef {
  thoughtId: string;
  createdAt: string;
}

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
  /** RFC 3339; for items, when it was said. */
  lastSeen: string | null;
  degree: number;
  /** For items: the recording it came from. */
  thoughtId: string | null;
}

export interface GraphData {
  nodes: NodeView[];
  edges: GraphEdge[];
}

/** A statement you made that matches a search (engine/search.rs). */
export interface SearchHit {
  /** Item or task id; item ids are also mind-map node ids. */
  id: string;
  kind: "task" | "idea" | "learning" | "question" | "decision";
  text: string;
  thoughtId: string | null;
  saidAt: string;
  score: number;
  /** Query words found verbatim, for highlighting. */
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
  /** Today's recordings that kept something. */
  thoughts: TodayThought[];
  /** Recordings today with nothing worth keeping. */
  noiseCount: number;
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
  items: Item[];
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
