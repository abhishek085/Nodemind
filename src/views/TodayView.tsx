import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Goal,
  ListeningStatus,
  Suggestion,
  FogInsight,
  ActivityItem,
  TranscriptChunk,
  LiveNote,
  GraphEdge,
  GraphNode,
} from "../types";
import SuggestionModal from "../components/SuggestionModal";

interface Props {
  transcript: string;
  listeningStatus: ListeningStatus;
  processingFinalNote?: boolean;
  language: "en" | "hi";
  triggerSummarize?: number;
  activityLog?: ActivityItem[];
}

export default function TodayView({
  transcript,
  listeningStatus,
  processingFinalNote = false,
  language,
  triggerSummarize,
  activityLog = [],
}: Props) {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [liveNotes, setLiveNotes] = useState<LiveNote[]>([]);
  const [summary, setSummary] = useState<string | null>(null);
  const [summarizing, setSummarizing] = useState(false);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [fogInsights] = useState<FogInsight[]>([]);
  const [suggestionsError, setSuggestionsError] = useState<string | null>(null);
  const [transcriptChunks, setTranscriptChunks] = useState<TranscriptChunk[]>([]);
  const [actionFeedback, setActionFeedback] = useState<string | null>(null);
  const [unprocessedNotesCount, setUnprocessedNotesCount] = useState(0);
  const [calendarModalOpen, setCalendarModalOpen] = useState(false);
  const [selectedSuggestionForCalendar, setSelectedSuggestionForCalendar] = useState<Suggestion | null>(null);
  const [refreshingSuggestions, setRefreshingSuggestions] = useState(false);
  const [graphNodes, setGraphNodes] = useState<GraphNode[]>([]);
  const [graphEdges, setGraphEdges] = useState<GraphEdge[]>([]);
  const [exportFeedback, setExportFeedback] = useState<string | null>(null);

  const fetchGoals = async () => {
    try {
      const g: Goal[] = await invoke("get_goals");
      setGoals(g);
    } catch {}
  };

  const fetchSavedSuggestions = async () => {
    try {
      const saved: Suggestion[] = await invoke("get_saved_suggestions");
      setSuggestions(saved);
    } catch {}
  };

  const fetchTranscriptChunks = async () => {
    try {
      const chunks: TranscriptChunk[] = await invoke("get_today_transcript_chunks");
      setTranscriptChunks(chunks);
    } catch {}
  };

  const fetchLiveNotes = async () => {
    try {
      const notes: LiveNote[] = await invoke("get_today_live_notes");
      setLiveNotes(notes);
    } catch {}
  };

  const fetchUnprocessedNotesCount = async () => {
    try {
      const count: number = await invoke("get_unprocessed_notes_count");
      setUnprocessedNotesCount(count);
    } catch {}
  };

  const fetchGraphData = async () => {
    try {
      const [nodes, edges] = await Promise.all([
        invoke<GraphNode[]>("get_graph_nodes"),
        invoke<GraphEdge[]>("get_graph_edges"),
      ]);
      setGraphNodes(nodes);
      setGraphEdges(edges);
    } catch {}
  };

  useEffect(() => {
    fetchGoals();
    fetchSavedSuggestions();
    fetchTranscriptChunks();
    fetchLiveNotes();
    fetchUnprocessedNotesCount();
    fetchGraphData();
    const id = setInterval(() => {
      fetchGoals();
      fetchSavedSuggestions();
      fetchTranscriptChunks();
      fetchLiveNotes();
      fetchUnprocessedNotesCount();
    }, 5000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    if (triggerSummarize && triggerSummarize > 0) summarize30();
  }, [triggerSummarize]);

  useEffect(() => {
    let unlistenSuggestions: UnlistenFn | null = null;
    fetchSavedSuggestions();

    const setup = async () => {
      unlistenSuggestions = await listen("suggestions-updated", () => {
        fetchSavedSuggestions();
      });
    };

    setup();

    return () => {
      if (unlistenSuggestions) {
        unlistenSuggestions();
      }
    };
  }, []);

  const summarize30 = async () => {
    setSummarizing(true);
    try {
      const s: string = await invoke("summarize_last_n_minutes", { minutes: 30 });
      setSummary(s);
    } catch {
      setSummary("Could not generate summary - is Ollama running?");
    } finally {
      setSummarizing(false);
    }
  };

  const handleRefreshSuggestions = async () => {
    setRefreshingSuggestions(true);
    setSuggestionsError(null);
    try {
      const msg: string = await invoke("refresh_suggestions");
      setActionFeedback(msg);
      setTimeout(() => setActionFeedback(null), 4000);
      await fetchSavedSuggestions();
      await fetchGoals();
      await fetchUnprocessedNotesCount();
    } catch (err) {
      setSuggestionsError(`Could not refresh suggestions — ${err instanceof Error ? err.message : String(err ?? "Ollama not reachable")}`);
    } finally {
      setRefreshingSuggestions(false);
    }
  };

  const acceptSuggestion = async (suggestion: Suggestion) => {
    if (!suggestion.id) return;
    try {
      if (suggestion.type === "task" || suggestion.type === "reflection") {
        await invoke("create_task", {
          title: suggestion.text,
          project: null,
          due_hint: null,
        });
      }
      await invoke("update_suggestion_status", { id: suggestion.id, status: "accepted" });
      setSuggestions((prev) => prev.filter((s) => s.id !== suggestion.id));
      setActionFeedback(
        suggestion.type === "reflection"
          ? "Mental loop added to Task List"
          : "Added to Task List"
      );
      setTimeout(() => setActionFeedback(null), 2000);
    } catch (err) {
      console.error("Error accepting suggestion:", err);
    }
  };

  const dismissSuggestion = async (id: string | undefined) => {
    if (!id) return;
    try {
      await invoke("update_suggestion_status", { id, status: "dismissed" });
      setSuggestions((prev) => prev.filter((s) => s.id !== id));
      setActionFeedback("Suggestion dismissed");
      setTimeout(() => setActionFeedback(null), 2000);
    } catch (err) {
      console.error("Error dismissing suggestion:", err);
    }
  };

  const openCalendarModal = (suggestion: Suggestion) => {
    setSelectedSuggestionForCalendar(suggestion);
    setCalendarModalOpen(true);
  };

  const handleCalendarSave = async () => {
    setCalendarModalOpen(false);
    if (selectedSuggestionForCalendar?.id) {
      await dismissSuggestion(selectedSuggestionForCalendar.id);
    }
    setActionFeedback("Focus session created");
    setTimeout(() => setActionFeedback(null), 2000);
  };

  type ChunkGroup = { startTime: string; chunks: TranscriptChunk[] };
  const GAP_MS = 5 * 60 * 1000;
  const transcriptGroups: ChunkGroup[] = [];
  for (const chunk of transcriptChunks) {
    const ts = new Date(chunk.timestamp).getTime();
    const lastGroup = transcriptGroups[transcriptGroups.length - 1];
    if (lastGroup) {
      const lastTs = new Date(lastGroup.chunks[lastGroup.chunks.length - 1].timestamp).getTime();
      if (ts - lastTs < GAP_MS) {
        lastGroup.chunks.push(chunk);
        continue;
      }
    }
    transcriptGroups.push({ startTime: chunk.timestamp, chunks: [chunk] });
  }

  const fmtTime = (iso: string) => {
    const d = new Date(iso);
    return d.toLocaleTimeString("en-US", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  };

  // Ghost goal detection: goals with no linked task node in graph (#22).
  const ghostGoals = useMemo(() => {
    const goalNodes = graphNodes.filter((n) => n.node_type === "goal");
    const taskNodes = graphNodes.filter((n) => n.node_type === "task");
    const linkedGoalIds = new Set<string>();
    for (const edge of graphEdges) {
      if (goalNodes.some((g) => g.id === edge.from_node) && taskNodes.some((t) => t.id === edge.to_node)) {
        linkedGoalIds.add(edge.from_node);
      }
      if (goalNodes.some((g) => g.id === edge.to_node) && taskNodes.some((t) => t.id === edge.from_node)) {
        linkedGoalIds.add(edge.to_node);
      }
    }
    return goalNodes.filter((g) => !linkedGoalIds.has(g.id)).slice(0, 3);
  }, [graphNodes, graphEdges]);

  // Markdown export (#18).
  const exportMarkdown = async () => {
    const lines: string[] = [];
    lines.push(`# Today — ${new Date().toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric" })}`);
    lines.push("");
    if (summary) {
      lines.push("## Summary");
      lines.push(summary);
      lines.push("");
    }
    if (liveNotes.length > 0) {
      lines.push("## Live Notes");
      for (const note of liveNotes) {
        lines.push(`### ${fmtTime(note.window_started_at)}`);
        lines.push(note.summary);
        for (const h of note.highlights) lines.push(`- **Highlight:** ${h}`);
        for (const t of note.tasks) lines.push(`- **Task:** ${t}`);
        lines.push("");
      }
    }
    if (activeGoals.length > 0) {
      lines.push("## Goals");
      for (const g of activeGoals) lines.push(`- [${g.horizon}] ${g.title}`);
      lines.push("");
    }
    if (dedupedTaskSuggestions.length > 0) {
      lines.push("## Task Suggestions");
      for (const s of dedupedTaskSuggestions) lines.push(`- ${s.text}`);
      lines.push("");
    }
    const md = lines.join("\n");
    try {
      await navigator.clipboard.writeText(md);
      setExportFeedback("Copied to clipboard!");
    } catch {
      setExportFeedback("Export failed — clipboard not available");
    }
    setTimeout(() => setExportFeedback(null), 3000);
  };

  const liveNoteCount = liveNotes.length;
  const visibleSuggestions = suggestions.filter((s) => s.status === undefined || s.status === "pending");

  // Separate suggestions by type
  const taskSuggestions = visibleSuggestions.filter((s) => s.type === "task");
  const reflectionSuggestions = visibleSuggestions.filter((s) => s.type === "reflection");

  // Deduplication: group by fingerprint or similar text, keep only most recent
  const deduplicateSuggestions = (suggs: Suggestion[]): Suggestion[] => {
    const seen = new Map<string, Suggestion>();
    for (const s of suggs) {
      const key = s.fingerprint || s.text.toLowerCase().trim();
      if (!seen.has(key)) {
        seen.set(key, s);
      } else {
        const existing = seen.get(key)!;
        if (s.created_at && existing.created_at && new Date(s.created_at) > new Date(existing.created_at)) {
          seen.set(key, s);
        }
      }
    }
    return Array.from(seen.values());
  };

  const dedupedTaskSuggestions = deduplicateSuggestions(taskSuggestions);
  const dedupedReflectionSuggestions = deduplicateSuggestions(reflectionSuggestions);
  const activeGoals = goals.filter((g) => g.status === "active");
  const sparkGoals = activeGoals.filter((g) => g.horizon === "spark");
  const milestoneGoals = activeGoals.filter((g) => g.horizon === "milestone");
  const northStarGoals = activeGoals.filter((g) => g.horizon === "north_star");

  return (
    <div className="view-today">
      <div className="view-header">
        <div>
          <h1 className="view-title">Today</h1>
          <p className="view-subtitle">
            {new Date().toLocaleDateString("en-US", {
              weekday: "long",
              month: "long",
              day: "numeric",
            })}
          </p>
        </div>
        <div className="header-actions">
          <button className="action-btn" onClick={handleRefreshSuggestions} disabled={refreshingSuggestions}>
            {refreshingSuggestions ? "Getting..." : "Get Suggestions"}
          </button>
          <button className="action-btn" onClick={summarize30} disabled={summarizing}>
            {summarizing ? "..." : "Summarize 30 min"}
          </button>
          <button className="action-btn" onClick={exportMarkdown} title="Export today's notes as Markdown">
            Export MD
          </button>
        </div>
      </div>

      {actionFeedback && (
        <div className="card feedback-card">
          <p className="feedback-text">{actionFeedback}</p>
        </div>
      )}

      {exportFeedback && (
        <div className="card feedback-card">
          <p className="feedback-text">{exportFeedback}</p>
        </div>
      )}

      {/* Ghost goal alert (#22): goals with no linked action nodes */}
      {ghostGoals.length > 0 && (
        <div className="card" style={{ borderColor: "rgba(245, 158, 11, 0.4)", background: "rgba(245, 158, 11, 0.06)" }}>
          <div className="card-label">Ghost Goals — no actions linked</div>
          {ghostGoals.map((g) => (
            <p key={g.id} className="hint-text" style={{ margin: "4px 0" }}>
              ⚠️ {g.label} — say a task that relates to this goal to link it
            </p>
          ))}
        </div>
      )}

      {/* Contextual voice hints (#14, #29) */}
      {listeningStatus === "off" && liveNotes.length === 0 && (
        <div className="card" style={{ borderColor: "rgba(99, 102, 241, 0.3)", background: "rgba(99, 102, 241, 0.05)" }}>
          <div className="card-label">Try saying…</div>
          <div className="voice-hints">
            {[
              "My goal is to ship the feature by Friday",
              "Remind me to follow up with Raghav tomorrow",
              "I'm stuck on the auth flow — not sure how to proceed",
              "I am taking a meeting now with the team about sprint planning",
              "Block two hours this afternoon for deep work",
            ].map((hint) => (
              <span key={hint} className="voice-hint-chip">{hint}</span>
            ))}
          </div>
        </div>
      )}

      {summary && (
        <div className="card summary-card">
          <div className="card-label">Summary - Last 30 min</div>
          <p className="summary-text">{summary}</p>
        </div>
      )}

      <section className="card live-notes-card">
        <div className="live-notes-header">
          <div>
            <div className="card-label">Live Notes - Today</div>
            <p className="live-notes-subtitle">Every 5-minute thought block adds a fresh note for today only.</p>
            {unprocessedNotesCount > 0 && (
              <p className="live-notes-subtitle">Pending analysis: {unprocessedNotesCount} note{unprocessedNotesCount === 1 ? "" : "s"}</p>
            )}
          </div>
          <span className="live-notes-count">{liveNoteCount}</span>
        </div>
        {liveNotes.length === 0 ? (
          <div className="empty-state live-notes-empty">No live notes yet. Keep talking and Nodemind will build today's running note.</div>
        ) : (
          <div className="live-notes-list">
            {liveNotes.map((note) => (
              <article key={note.id} className="live-note-item">
                <div className="live-note-meta">
                  <span className="live-note-time">{fmtTime(note.window_started_at)}</span>
                  <span className="live-note-window">5 min block</span>
                </div>
                <ul className="live-note-bullets">
                  <li><strong>Summary:</strong> {note.summary}</li>
                  {note.highlights.map((item, index) => (
                    <li key={`${note.id}-highlight-${index}`}><strong>Highlight:</strong> {item}</li>
                  ))}
                  {note.ideas.map((item, index) => (
                    <li key={`${note.id}-idea-${index}`}><strong>Idea:</strong> {item}</li>
                  ))}
                  {note.tasks.map((item, index) => (
                    <li key={`${note.id}-task-${index}`}><strong>Task:</strong> {item}</li>
                  ))}
                </ul>
              </article>
            ))}
          </div>
        )}
      </section>

      <div className="card status-card">
        <div className="listening-status-row">
          <div className={`status-orb orb-${listeningStatus}`} />
          <span className="status-label">
            {processingFinalNote
              ? "Listening stopped - processing final note in background"
              : listeningStatus === "listening"
              ? `Listening in ${language === "hi" ? "Hindi/Hinglish" : "English"}`
              : listeningStatus === "idle"
                ? "Idle - no speech detected for 60s"
                : "Not listening"}
          </span>
        </div>
        {processingFinalNote && (
          <div className="processing-note-inline">
            <span className="processing-dot" />
            <span className="hint-text">Processing final note in background...</span>
          </div>
        )}
        {listeningStatus === "listening" && transcript && (
          <div className="live-preview">{transcript.slice(-200)}</div>
        )}
      </div>

      {transcriptChunks.length > 0 && (
        <details className="card transcript-panel">
          <summary className="transcript-summary">
            <span>Raw Transcript</span>
            <span className="transcript-word-count">
              {transcriptChunks.reduce((n, c) => n + c.text.trim().split(/\s+/).length, 0)} words - {transcriptChunks.length} chunks
            </span>
          </summary>
          <div className="transcript-body">
            {transcriptGroups.map((group, gi) => (
              <div key={gi} className="transcript-group">
                {gi > 0 && (
                  <div className="transcript-gap-marker">
                    -- {(() => {
                      const prev = new Date(transcriptGroups[gi - 1].chunks[transcriptGroups[gi - 1].chunks.length - 1].timestamp).getTime();
                      const curr = new Date(group.startTime).getTime();
                      const mins = Math.round((curr - prev) / 60000);
                      return `${mins} min gap`;
                    })()} --
                  </div>
                )}
                {group.chunks.map((chunk) => (
                  <div key={chunk.id} className="transcript-chunk-row">
                    <span className="transcript-time">[{fmtTime(chunk.timestamp)}]</span>
                    <span className="transcript-chunk-text">{chunk.text}</span>
                  </div>
                ))}
              </div>
            ))}
            <button
              className="action-btn transcript-clear-btn"
              onClick={async () => { await invoke("clear_transcript"); }}
            >
              Clear
            </button>
          </div>
        </details>
      )}

      {activityLog.length > 0 && (
        <details className="card activity-panel" open>
          <summary className="activity-summary">
            <span className="activity-title">Live Captures</span>
            <span className="activity-count">{activityLog.length} entries</span>
          </summary>
          <div className="activity-feed">
            {activityLog.map((item, i) => (
              <div key={i} className="activity-entry">
                <span className="activity-time">{item.time}</span>
                <div className="activity-chips">
                  {item.tasks.map((t, j) => (
                    <span key={`t${j}`} className="activity-chip chip-task">✓ {t}</span>
                  ))}
                  {item.fog.map((f, j) => (
                    <span key={`f${j}`} className="activity-chip chip-fog">⚡ {f}</span>
                  ))}
                  {item.topics.map((tp, j) => (
                    <span key={`tp${j}`} className="activity-chip chip-topic"># {tp}</span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </details>
      )}

      <div className="today-grid">
        <div className="today-col">
          <div className="col-header">
            <h2 className="col-title">Goals</h2>
            <span className="col-count">{activeGoals.length}</span>
          </div>

          {activeGoals.length === 0 ? (
            <div className="empty-state">Say "my goal is..." to add goals</div>
          ) : (
            <>
              <div className="card-label" style={{ marginBottom: "0.5rem" }}>Sparks (Short Term)</div>
              {sparkGoals.length === 0 ? (
                <div className="empty-state" style={{ marginBottom: "0.75rem" }}>No short-term goals yet.</div>
              ) : (
                sparkGoals.map((goal) => (
                  <div key={goal.id} className="goal-card">
                    <div className="goal-title">{goal.title}</div>
                    {goal.description && <div className="goal-desc">{goal.description}</div>}
                  </div>
                ))
              )}

              <div className="card-label" style={{ marginTop: "0.75rem", marginBottom: "0.5rem" }}>Milestones (Medium Term)</div>
              {milestoneGoals.length === 0 ? (
                <div className="empty-state" style={{ marginBottom: "0.75rem" }}>No medium-term goals yet.</div>
              ) : (
                milestoneGoals.map((goal) => (
                  <div key={goal.id} className="goal-card">
                    <div className="goal-title">{goal.title}</div>
                    {goal.description && <div className="goal-desc">{goal.description}</div>}
                  </div>
                ))
              )}

              <div className="card-label" style={{ marginTop: "0.75rem", marginBottom: "0.5rem" }}>North Stars (Long Term)</div>
              {northStarGoals.length === 0 ? (
                <div className="empty-state">No long-term goals yet.</div>
              ) : (
                northStarGoals.map((goal) => (
                  <div key={goal.id} className="goal-card">
                    <div className="goal-title">{goal.title}</div>
                    {goal.description && <div className="goal-desc">{goal.description}</div>}
                  </div>
                ))
              )}
            </>
          )}
        </div>

        <div className="today-col">
          {suggestionsError && (
            <div className="card summary-card" style={{ marginTop: "1rem", color: "var(--color-warn, #e07b39)" }}>
              <div className="card-label">Suggestions</div>
              <p className="summary-text">{suggestionsError}</p>
            </div>
          )}

          {dedupedTaskSuggestions.length > 0 && (
            <section className="suggestions-section task-suggestions">
              <div className="col-header">
                <h2 className="col-title">Task Suggestions</h2>
                <span className="col-count">{dedupedTaskSuggestions.length}</span>
              </div>
              {dedupedTaskSuggestions.map((s, i) => (
                <div key={s.id ?? i} className={`suggestion-card prio-${s.priority}`}>
                  <div className="suggestion-header">
                    <span className="suggestion-type">Task</span>
                    {s.confidence && (
                      <span className="suggestion-confidence">{Math.round(s.confidence * 100)}%</span>
                    )}
                  </div>
                  <p className="suggestion-text">{s.text}</p>
                  <div className="suggestion-actions">
                    <button className="sug-btn sug-accept" onClick={() => acceptSuggestion(s)}>
                      Add to List
                    </button>
                    <button className="sug-btn sug-calendar" onClick={() => openCalendarModal(s)}>
                      📅 Schedule
                    </button>
                    <button className="sug-btn sug-dismiss" onClick={() => dismissSuggestion(s.id)}>
                      Dismiss
                    </button>
                  </div>
                </div>
              ))}
            </section>
          )}

          {dedupedReflectionSuggestions.length > 0 && (
            <section className="suggestions-section reflection-suggestions">
              <div className="col-header">
                <h2 className="col-title">Mental Loops</h2>
                <span className="col-count">{dedupedReflectionSuggestions.length}</span>
              </div>
              {dedupedReflectionSuggestions.map((s, i) => (
                <div key={s.id ?? i} className="reflection-card">
                  <div className="reflection-header">
                    <span className="reflection-icon">⚠️</span>
                    <div>
                      <div className="reflection-title">{s.text}</div>
                      {s.context && (
                        <div className="reflection-context">{s.context}</div>
                      )}
                    </div>
                  </div>
                  <div className="suggestion-actions">
                    <button className="sug-btn sug-accept" onClick={() => acceptSuggestion(s)}>
                      Prioritize
                    </button>
                    <button className="sug-btn sug-calendar" onClick={() => openCalendarModal(s)}>
                      📅 Schedule
                    </button>
                    <button className="sug-btn sug-dismiss" onClick={() => dismissSuggestion(s.id)}>
                      Dismiss
                    </button>
                  </div>
                </div>
              ))}
            </section>
          )}

          {dedupedTaskSuggestions.length === 0 && dedupedReflectionSuggestions.length === 0 && (
            <div className="empty-state" style={{ marginTop: "0.5rem" }}>
              No suggestions right now. Keep talking; Nodemind will validate new candidates.
            </div>
          )}

          {fogInsights.length > 0 && (
            <section className="suggestions-section fog-suggestions">
              <div className="col-header" style={{ marginTop: "1.5rem" }}>
                <h2 className="col-title">Fog Insights</h2>
                <span className="col-count">{fogInsights.length}</span>
              </div>
              {fogInsights.map((fi, i) => (
                <div key={i} className="fog-insight-card">
                  <div className="fog-pattern">{fi.pattern}</div>
                  <p className="fog-insight-text">{fi.insight}</p>
                  <div className="fog-intervention">💡 {fi.intervention}</div>
                </div>
              ))}
            </section>
          )}
        </div>
      </div>

      {selectedSuggestionForCalendar && (
        <SuggestionModal
          suggestion={selectedSuggestionForCalendar}
          isOpen={calendarModalOpen}
          onClose={() => setCalendarModalOpen(false)}
          onSave={handleCalendarSave}
        />
      )}
    </div>
  );
}
