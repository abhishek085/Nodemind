import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";

type FogStat = [string, number];

interface FogDetail {
  id: string;
  chunk_id: string;
  tag: string;
  context: string;
  resolution: string;
  strength: number;
  markers: string;
  timestamp: string;
}

interface GraphNodeLite {
  id: string;
  data?: string | null;
}

const FOG_DESCRIPTIONS: Record<string, { label: string; color: string; tip: string }> = {
  overthinking: {
    label: "Overthinking",
    color: "#f59e0b",
    tip: "Try a 2-minute timer. Write the one thing blocking you, then start.",
  },
  "context switching": {
    label: "Context Switching",
    color: "#ef4444",
    tip: "Block 30-minute slots — only one task per slot. No tab-switching.",
  },
  stuck: {
    label: "Stuck",
    color: "#8b5cf6",
    tip: "Shrink the task to its smallest possible first step. Do just that.",
  },
  distracted: {
    label: "Distracted",
    color: "#06b6d4",
    tip: "Close everything except one app. Use Focus mode.",
  },
  unclear: {
    label: "Unclear / Foggy",
    color: "#64748b",
    tip: "Write down the question you're actually trying to answer.",
  },
  unclarity: {
    label: "Unclarity",
    color: "#64748b",
    tip: "Name the owner, deadline, and first action in one sentence.",
  },
  abstraction: {
    label: "Abstraction",
    color: "#0ea5e9",
    tip: "Convert this idea into one concrete object you can change today.",
  },
  circular: {
    label: "Circular Loop",
    color: "#f97316",
    tip: "What assumption are you repeating that might be wrong?",
  },
  overwhelmed: {
    label: "Overwhelmed",
    color: "#dc2626",
    tip: "Pick one priority and explicitly defer everything else.",
  },
};

export default function FogView() {
  const [stats, setStats] = useState<FogStat[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [fogDetails, setFogDetails] = useState<FogDetail[]>([]);
  const [bucketAlert, setBucketAlert] = useState<string | null>(null);

  const loadFogDetails = async () => {
    try {
      const details: FogDetail[] = await invoke("get_fog_details", { hours: 24 });
      setFogDetails(details || []);
    } catch {
      setFogDetails([]);
    }
  };
  useEffect(() => {
    (async () => {
      try {
        const s: FogStat[] = await invoke("get_fog_stats_cmd");
        setStats(s);
        await loadFogDetails();
        const nodes: GraphNodeLite[] = await invoke("get_graph_nodes");
        const counts = { Work: 0, Wellness: 0, Social: 0, Growth: 0 };
        for (const node of nodes || []) {
          if (!node.data) continue;
          try {
            const meta = JSON.parse(node.data) as { category?: string };
            const c = (meta.category || "").trim();
            if (c === "Work" || c === "Wellness" || c === "Social" || c === "Growth") {
              counts[c] += 1;
            }
          } catch {
            // Ignore malformed metadata payloads.
          }
        }
        const total = counts.Work + counts.Wellness + counts.Social + counts.Growth;
        if (total > 0) {
          const workPct = (counts.Work / total) * 100;
          if (workPct >= 90 && counts.Growth === 0) {
            setBucketAlert("Your map is heavily Work-weighted and has no Growth nodes today. Consider a short reflection block to restore balance.");
          } else {
            setBucketAlert(null);
          }
        }
      } catch {}
    })();
    const id = setInterval(async () => {
      try {
        const s: FogStat[] = await invoke("get_fog_stats_cmd");
        setStats(s);
        await loadFogDetails();
        const nodes: GraphNodeLite[] = await invoke("get_graph_nodes");
        const counts = { Work: 0, Wellness: 0, Social: 0, Growth: 0 };
        for (const node of nodes || []) {
          if (!node.data) continue;
          try {
            const meta = JSON.parse(node.data) as { category?: string };
            const c = (meta.category || "").trim();
            if (c === "Work" || c === "Wellness" || c === "Social" || c === "Growth") {
              counts[c] += 1;
            }
          } catch {
            // Ignore malformed metadata payloads.
          }
        }
        const total = counts.Work + counts.Wellness + counts.Social + counts.Growth;
        if (total > 0) {
          const workPct = (counts.Work / total) * 100;
          if (workPct >= 90 && counts.Growth === 0) {
            setBucketAlert("Your map is heavily Work-weighted and has no Growth nodes today. Consider a short reflection block to restore balance.");
          } else {
            setBucketAlert(null);
          }
        }
      } catch {}
    }, 10000);
    return () => clearInterval(id);
  }, []);

  const normalizedTag = (tag: string) => tag.toLowerCase().split("_").join(" ").trim();

  const groupedSignals = useMemo(() => {
    const map = new Map<string, FogDetail[]>();
    for (const entry of fogDetails) {
      const key = normalizedTag(entry.tag || "unclear");
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(entry);
    }
    return Array.from(map.entries())
      .map(([tag, entries]) => ({
        tag,
        entries: entries.sort((a, b) => +new Date(b.timestamp) - +new Date(a.timestamp)),
      }))
      .sort((a, b) => b.entries.length - a.entries.length);
  }, [fogDetails]);

  const selectedGroup = selected
    ? groupedSignals.find((g) => g.tag === selected) ?? null
    : groupedSignals[0] ?? null;

  const totalSignals = groupedSignals.reduce((sum, g) => sum + g.entries.length, 0);
  const fallbackTotal = stats.reduce((sum, [, n]) => sum + n, 0);
  const total = totalSignals || fallbackTotal;

  const describeWindowMinutes = (entries: FogDetail[]): number => {
    if (entries.length < 2) return 0;
    const newest = +new Date(entries[0].timestamp);
    const oldest = +new Date(entries[entries.length - 1].timestamp);
    return Math.max(1, Math.round((newest - oldest) / (1000 * 60)));
  };

  return (
    <div className="view-fog">
      <div className="view-header">
        <div>
          <h1 className="view-title">Mental Fog</h1>
          <p className="view-subtitle">
            {total > 0
              ? `${total} cognitive signals captured from live thinking`
              : "No fog patterns detected yet — start listening!"}
          </p>
        </div>
      </div>

      {bucketAlert && (
        <div className="card" style={{ borderColor: "rgba(245, 158, 11, 0.5)", background: "rgba(245, 158, 11, 0.08)" }}>
          <p className="fog-tip-text">Well-being alert: {bucketAlert}</p>
        </div>
      )}

      {groupedSignals.length === 0 ? (
        <div className="card empty-fog-card">
          <div className="fog-empty-icon">🌫️</div>
          <p className="empty-state">
            Fog patterns are detected automatically as you think out loud.
            <br />
            Start listening and talk through your work.
          </p>
        </div>
      ) : (
        <div className="fog-layout">
          {/* Signal group overview */}
          <div className="fog-chips">
            {groupedSignals.map(({ tag, entries }) => {
              const info = FOG_DESCRIPTIONS[tag] ?? FOG_DESCRIPTIONS.unclear;
              const color = info?.color ?? "#94a3b8";
              const count = entries.length;
              const pct = Math.round((count / Math.max(total, 1)) * 100);
              const recentContext = entries[0]?.context || "No context captured";
              return (
                <button
                  key={tag}
                  className={`fog-chip${selected === tag ? " fog-chip-selected" : ""}`}
                  style={{ "--fog-color": color } as React.CSSProperties}
                  onClick={() => setSelected(selected === tag ? null : tag)}
                >
                  <span className="fog-chip-label">{info?.label ?? tag}</span>
                  <span className="fog-chip-count">{count}</span>
                  <div className="fog-chip-bar">
                    <div className="fog-chip-fill" style={{ width: `${pct}%` }} />
                  </div>
                  <p className="fog-tip-text" style={{ marginTop: 8, textAlign: "left" }}>
                    {recentContext}
                  </p>
                </button>
              );
            })}
          </div>

          {/* Detail panel */}
          {selectedGroup && (
            <div className="fog-detail card">
              {(() => {
                const info = FOG_DESCRIPTIONS[selectedGroup.tag] ?? FOG_DESCRIPTIONS.unclear;
                const count = selectedGroup.entries.length;
                const pct = Math.round((count / Math.max(total, 1)) * 100);
                const latest = selectedGroup.entries[0];
                const mins = describeWindowMinutes(selectedGroup.entries.slice(0, 5));
                const headline = mins > 0
                  ? `You've been ${info?.label ?? selectedGroup.tag} about "${latest?.context || "this topic"}" for the last ${mins} minutes.`
                  : `You've been ${info?.label ?? selectedGroup.tag} about "${latest?.context || "this topic"}" recently.`;
                return (
                  <>
                    <div className="fog-detail-header">
                      <h2 className="fog-detail-title">{info?.label ?? selectedGroup.tag}</h2>
                      <span className="fog-detail-count">{count}× detected ({pct}%)</span>
                    </div>

                    <p className="fog-tip-text" style={{ marginBottom: 12 }}>{headline}</p>

                    <div className="fog-progress-wrap">
                      <div className="fog-progress-bar">
                        <div
                          className="fog-progress-fill"
                          style={{
                            width: `${pct}%`,
                            background: info?.color ?? "#94a3b8",
                          }}
                        />
                      </div>
                    </div>

                    <div className="fog-tip-card">
                      <div className="fog-tip-label">Clarity Action</div>
                      <p className="fog-tip-text">
                        {latest?.resolution || info?.tip || "Keep note of when this occurs."}
                      </p>

                      {/* Show recent moments with context */}
                      <div className="fog-entries">
                        <div className="fog-entries-label">Recent moments:</div>
                        {selectedGroup.entries.length === 0 ? (
                          <p className="fog-entries-empty">No recorded entries yet</p>
                        ) : (
                          <div className="fog-entries-list">
                            {selectedGroup.entries.slice(0, 5).map((entry) => (
                              <div key={entry.id} className="fog-entry-card">
                                <div className="fog-entry-header">
                                  <span className="fog-entry-time">
                                    {new Date(entry.timestamp).toLocaleTimeString()}
                                  </span>
                                  <span className="fog-entry-strength">
                                    Intensity: {(entry.strength * 100).toFixed(0)}%
                                  </span>
                                </div>
                                <p className="fog-tip-text" style={{ margin: "8px 0" }}>
                                  {entry.context || "No context captured"}
                                </p>
                                <div className="fog-entry-markers">
                                  <span className="fog-marker-tag">{entry.tag}</span>
                                </div>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                  </>
                );
              })()}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
