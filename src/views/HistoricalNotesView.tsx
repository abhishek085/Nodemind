import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { HistoricalNote } from "../types";

interface Props {
  onOpenToday: () => void;
}

export default function HistoricalNotesView({ onOpenToday }: Props) {
  const [notes, setNotes] = useState<HistoricalNote[]>([]);
  const [dateFilter, setDateFilter] = useState("");

  useEffect(() => {
    (async () => {
      try {
        const result: HistoricalNote[] = await invoke("get_historical_notes");
        setNotes(result);
      } catch {
        setNotes([]);
      }
    })();
  }, []);

  const filteredNotes = useMemo(() => {
    if (!dateFilter) return notes;
    return notes.filter((note) => note.day_key.includes(dateFilter));
  }, [dateFilter, notes]);

  return (
    <div className="view-notes-history">
      <div className="view-header">
        <div>
          <h1 className="view-title">Historical Notes</h1>
          <p className="view-subtitle">Search daily note rollups by date</p>
        </div>
        <div className="header-actions">
          <button className="action-btn" onClick={onOpenToday}>
            Open Today Live Notes
          </button>
        </div>
      </div>

      <div className="card notes-filter-card">
        <div className="settings-row">
          <div className="settings-label">Date</div>
          <div className="settings-value">
            <input
              className="qa-input"
              type="date"
              value={dateFilter}
              onChange={(event) => setDateFilter(event.target.value)}
            />
            {dateFilter && (
              <button className="action-btn ml-1" onClick={() => setDateFilter("")}>
                Clear
              </button>
            )}
          </div>
        </div>
      </div>

      {filteredNotes.length === 0 ? (
        <div className="empty-state">No historical notes found for that date.</div>
      ) : (
        <div className="notes-history-list">
          {filteredNotes.map((note) => (
            <section key={note.id} className="card historical-note-card">
              <div className="historical-note-header">
                <div>
                  <div className="card-label">{note.day_key}</div>
                  <h2 className="historical-note-title">{note.note_count} live note blocks</h2>
                </div>
              </div>
              <p className="summary-text">{note.summary}</p>

              {note.highlights.length > 0 && (
                <div className="historical-note-section">
                  <div className="historical-note-label">Highlights</div>
                  <ul className="historical-note-list">
                    {note.highlights.map((item, index) => (
                      <li key={`${note.id}-highlight-${index}`}>{item}</li>
                    ))}
                  </ul>
                </div>
              )}

              {note.ideas.length > 0 && (
                <div className="historical-note-section">
                  <div className="historical-note-label">Ideas</div>
                  <ul className="historical-note-list">
                    {note.ideas.map((item, index) => (
                      <li key={`${note.id}-idea-${index}`}>{item}</li>
                    ))}
                  </ul>
                </div>
              )}

              {note.tasks.length > 0 && (
                <div className="historical-note-section">
                  <div className="historical-note-label">Tasks</div>
                  <ul className="historical-note-list">
                    {note.tasks.map((item, index) => (
                      <li key={`${note.id}-task-${index}`}>{item}</li>
                    ))}
                  </ul>
                </div>
              )}
            </section>
          ))}
        </div>
      )}
    </div>
  );
}