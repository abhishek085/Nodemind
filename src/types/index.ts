export interface Task {
  id: string;
  title: string;
  project?: string | null;
  due_hint?: string | null;
  done: boolean;
  created_at: string;
  source: "ambient" | "command" | "manual";
  status?: "official" | "archived";
}

export interface Goal {
  id: string;
  title: string;
  description?: string | null;
  created_at: string;
  status: "active" | "completed" | "archived";
  horizon: "spark" | "milestone" | "north_star";
}

export interface GraphNode {
  id: string;
  node_type: "person" | "project" | "goal" | "topic" | "fog_pattern" | "task";
  label: string;
  data?: string | null;
  created_at: string;
}

export interface GraphEdge {
  id: string;
  from_node: string;
  to_node: string;
  edge_type: "blocks" | "relates_to" | "works_with" | "mentions" | "triggers";
  weight: number;
  last_updated: string;
}

export interface Meeting {
  id: string;
  title: string;
  person?: string | null;
  started_at: string;
  ended_at?: string | null;
  summary?: string | null;
  action_items?: string | null;
  transcript?: string | null;
}

export interface FocusSession {
  id: string;
  title: string;
  date: string;        // YYYY-MM-DD
  start_time: string;  // HH:MM
  end_time: string;    // HH:MM
  notes?: string | null;
  created_at: string;
}

export interface Suggestion {
  id?: string;
  type: "task" | "reflection" | "focus" | "goal";
  text: string;
  priority: "high" | "medium" | "low";
  status?: "pending" | "accepted" | "dismissed";
  created_at?: string;
  fingerprint?: string;    // For deduplication of similar suggestions
  confidence?: number;     // 0-1 confidence score
  context?: string;        // Optional context (e.g., for reflection: frequency count or problem statement)
}

export interface FogInsight {
  pattern: string;
  insight: string;
  intervention: string;
}

export interface LiveNote {
  id: string;
  day_key: string;
  window_started_at: string;
  window_ended_at: string;
  summary: string;
  highlights: string[];
  ideas: string[];
  tasks: string[];
  fingerprint: string;
  created_at: string;
}

export interface HistoricalNote {
  id: string;
  day_key: string;
  summary: string;
  highlights: string[];
  ideas: string[];
  tasks: string[];
  note_count: number;
  created_at: string;
}

export type View = "today" | "tasks" | "map" | "meetings" | "fog" | "calendar" | "notes" | "settings";

export type ListeningStatus = "off" | "listening" | "idle";

export interface ActivityItem {
  time: string;         // "HH:MM" when the chunk was processed
  tasks: string[];      // task titles found
  fog: string[];        // fog signal strings
  topics: string[];     // topic strings
}

export interface TranscriptChunk {
  id: string;
  text: string;
  language: string;
  timestamp: string;     // ISO 8601 / RFC 3339
  meeting_id?: string | null;
  fog_tags?: string | null;
}
