import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Meeting } from "../types";

type Tab = "meetings" | "people";

export default function MeetingsView() {
  const [tab, setTab] = useState<Tab>("meetings");
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [selected, setSelected] = useState<Meeting | null>(null);
  const [activeMeetingId, setActiveMeetingId] = useState<string | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const [newPerson, setNewPerson] = useState("");
  const [actionItemsParsed, setActionItemsParsed] = useState<string[]>([]);
  const [selectedPerson, setSelectedPerson] = useState<string | null>(null);

  const fetchMeetings = async () => {
    try {
      const m: Meeting[] = await invoke("get_meetings");
      setMeetings(m);
      const active: string | null = await invoke("get_active_meeting");
      setActiveMeetingId(active);
    } catch {}
  };

  useEffect(() => {
    fetchMeetings();
    const id = setInterval(fetchMeetings, 5000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    if (selected?.action_items) {
      try {
        const parsed = JSON.parse(selected.action_items);
        setActionItemsParsed(Array.isArray(parsed) ? parsed : []);
      } catch {
        setActionItemsParsed([]);
      }
    } else {
      setActionItemsParsed([]);
    }
  }, [selected]);

  const startMeeting = async () => {
    if (!newTitle.trim()) return;
    await invoke("start_meeting", {
      title: newTitle.trim(),
      person: newPerson.trim() || null,
    });
    setNewTitle("");
    setNewPerson("");
    fetchMeetings();
  };

  const endMeeting = async () => {
    await invoke("end_meeting");
    fetchMeetings();
  };

  // ── People derived from meetings ────────────────────────────────────────────
  const peopleMap = useMemo(() => {
    const map: Record<string, Meeting[]> = {};
    for (const m of meetings) {
      if (m.person) {
        if (!map[m.person]) map[m.person] = [];
        map[m.person].push(m);
      }
    }
    return map;
  }, [meetings]);

  const personMeetings = selectedPerson ? (peopleMap[selectedPerson] ?? []) : [];
  const lastMeeting = personMeetings[0] ?? null; // already sorted DESC

  const formatDuration = (start: string, end?: string | null) => {
    const s = new Date(start);
    const e = end ? new Date(end) : new Date();
    const mins = Math.round((e.getTime() - s.getTime()) / 60000);
    if (mins < 60) return `${mins}m`;
    return `${Math.floor(mins / 60)}h ${mins % 60}m`;
  };

  const formatDate = (iso: string) => {
    return new Date(iso).toLocaleDateString("en-US", {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  return (
    <div className="view-meetings">
      <div className="view-header">
        <div>
          <h1 className="view-title">Meetings & People</h1>
          <p className="view-subtitle">{meetings.length} meetings recorded</p>
        </div>
      </div>

      {/* Active meeting banner */}
      {activeMeetingId && (
        <div className="active-meeting-banner">
          <span className="meeting-live-dot pulse" />
          <span>Meeting in progress</span>
          <button className="action-btn action-btn-danger" onClick={endMeeting}>
            End Meeting
          </button>
        </div>
      )}

      {/* Start meeting form */}
      {!activeMeetingId && (
        <div className="card start-meeting-card">
          <div className="card-label">Start a Meeting</div>
          <div className="meeting-form-row">
            <input
              className="qa-input"
              placeholder='Topic (e.g. "Nokast partnership")'
              value={newTitle}
              onChange={(e) => setNewTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && startMeeting()}
            />
            <input
              className="qa-input qa-small"
              placeholder="Person"
              value={newPerson}
              onChange={(e) => setNewPerson(e.target.value)}
            />
            <button className="action-btn" onClick={startMeeting}>
              Start
            </button>
          </div>
          <p className="hint-text">
            Or say "I am taking a meeting now with [name] about [topic]"
          </p>
        </div>
      )}

      {/* ── Tabs ────────────────────────────────────────── */}
      <div className="meetings-tabs">
        <button
          className={`meetings-tab${tab === "meetings" ? " meetings-tab-active" : ""}`}
          onClick={() => setTab("meetings")}
        >
          Meetings
          <span className="tab-count">{meetings.length}</span>
        </button>
        <button
          className={`meetings-tab${tab === "people" ? " meetings-tab-active" : ""}`}
          onClick={() => { setTab("people"); setSelected(null); }}
        >
          People
          <span className="tab-count">{Object.keys(peopleMap).length}</span>
        </button>
      </div>

      {/* ── Meetings tab ─────────────────────────────────── */}
      {tab === "meetings" && (
        <div className="meetings-layout">
          <div className="meetings-list">
            {meetings.length === 0 ? (
              <div className="empty-state">No meetings yet</div>
            ) : (
              meetings.map((m) => (
                <div
                  key={m.id}
                  className={`meeting-item${selected?.id === m.id ? " meeting-selected" : ""}${!m.ended_at ? " meeting-active" : ""}`}
                  onClick={() => setSelected(m)}
                >
                  <div className="meeting-item-header">
                    <span className="meeting-title">{m.title}</span>
                    {!m.ended_at && <span className="live-badge">LIVE</span>}
                  </div>
                  {m.person && (
                    <div className="meeting-person">with {m.person}</div>
                  )}
                  <div className="meeting-meta">
                    <span>{formatDate(m.started_at)}</span>
                    <span className="meeting-duration">
                      {formatDuration(m.started_at, m.ended_at)}
                    </span>
                  </div>
                </div>
              ))
            )}
          </div>

          {selected && (
            <div className="meeting-detail card">
              <div className="detail-header">
                <h2 className="detail-title">{selected.title}</h2>
                {selected.person && (
                  <div className="detail-person">with {selected.person}</div>
                )}
                <div className="detail-meta">
                  {formatDate(selected.started_at)}
                  {selected.ended_at &&
                    ` — ${formatDate(selected.ended_at)} (${formatDuration(
                      selected.started_at,
                      selected.ended_at,
                    )})`}
                </div>
              </div>

              {selected.summary ? (
                <div className="detail-section">
                  <div className="section-label">Summary</div>
                  <p className="summary-text">{selected.summary}</p>
                </div>
              ) : (
                <div className="detail-section">
                  <p className="hint-text">
                    {selected.ended_at
                      ? "No summary generated yet"
                      : "Notes will be generated when the meeting ends"}
                  </p>
                </div>
              )}

              {actionItemsParsed.length > 0 && (
                <div className="detail-section">
                  <div className="section-label">Action Items</div>
                  <ul className="action-items-list">
                    {actionItemsParsed.map((ai, i) => (
                      <li key={i} className="action-item">
                        {ai}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {selected.transcript && (
                <div className="detail-section">
                  <div className="section-label">Transcript</div>
                  <div className="transcript-scroll">{selected.transcript}</div>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* ── People tab ────────────────────────────────────── */}
      {tab === "people" && (
        <div className="meetings-layout">
          <div className="meetings-list">
            {Object.keys(peopleMap).length === 0 ? (
              <div className="empty-state">
                No people recorded yet — they appear automatically when meetings have a person name.
              </div>
            ) : (
              Object.entries(peopleMap).map(([name, pMeetings]) => (
                <div
                  key={name}
                  className={`meeting-item${selectedPerson === name ? " meeting-selected" : ""}`}
                  onClick={() => setSelectedPerson(name === selectedPerson ? null : name)}
                >
                  <div className="meeting-item-header">
                    <span className="meeting-title person-name">{name}</span>
                    <span className="person-count">{pMeetings.length} meeting{pMeetings.length !== 1 ? "s" : ""}</span>
                  </div>
                  {pMeetings[0] && (
                    <div className="meeting-meta">
                      Last: {formatDate(pMeetings[0].started_at)}
                    </div>
                  )}
                </div>
              ))
            )}
          </div>

          {selectedPerson && (
            <div className="meeting-detail card">
              <div className="detail-header">
                <h2 className="detail-title">{selectedPerson}</h2>
                <div className="detail-meta">
                  {personMeetings.length} meeting{personMeetings.length !== 1 ? "s" : ""} recorded
                </div>
              </div>

              {/* Latest meeting summary */}
              {lastMeeting?.summary && (
                <div className="detail-section">
                  <div className="section-label">Last Meeting Summary</div>
                  <p className="summary-text">{lastMeeting.summary}</p>
                </div>
              )}

              {/* Meeting timeline */}
              <div className="detail-section">
                <div className="section-label">Meeting History</div>
                <div className="person-timeline">
                  {personMeetings.map((m) => (
                    <div key={m.id} className="person-timeline-item">
                      <div className="person-tl-dot" />
                      <div className="person-tl-body">
                        <span className="person-tl-title">{m.title}</span>
                        <span className="person-tl-date">{formatDate(m.started_at)}</span>
                        {(() => {
                          const items: string[] = (() => {
                            try { const p = JSON.parse(m.action_items ?? ""); return Array.isArray(p) ? p : []; } catch { return []; }
                          })();
                          return items.length > 0 ? (
                            <ul className="person-tl-actions">
                              {items.slice(0, 3).map((ai, i) => <li key={i}>{ai}</li>)}
                              {items.length > 3 && <li className="tl-more">+{items.length - 3} more</li>}
                            </ul>
                          ) : null;
                        })()}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}


