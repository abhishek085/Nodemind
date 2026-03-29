import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Suggestion } from "../types";

interface Props {
  suggestion: Suggestion;
  isOpen: boolean;
  onClose: () => void;
  onSave?: () => void;
}

export default function SuggestionModal({
  suggestion,
  isOpen,
  onClose,
  onSave,
}: Props) {
  const [title, setTitle] = useState(suggestion.text);
  const [date, setDate] = useState(new Date().toISOString().split("T")[0]);
  const [startTime, setStartTime] = useState("09:00");
  const [endTime, setEndTime] = useState("10:00");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen) return null;

  const handleSave = async () => {
    setSaving(true);
    setError(null);

    try {
      if (!title.trim()) {
        setError("Title is required");
        setSaving(false);
        return;
      }
      if (endTime <= startTime) {
        setError("End time must be after start time");
        setSaving(false);
        return;
      }

      // Create focus session with camelCase (Tauri serializes to snake_case)
      const focusSession = {
        title: title.trim(),
        date,
        startTime,
        endTime,
      };

      const created = await invoke<boolean>("create_focus_session", focusSession);
      if (!created) {
        setError("That time is already booked. Please choose another slot.");
        setSaving(false);
        return;
      }

      // Optionally create a task as well
      await invoke("create_task", {
        title: title.trim(),
        project: null,
        due_hint: date,
      });

      // Close modal and trigger refresh
      setTitle(suggestion.text);
      setDate(new Date().toISOString().split("T")[0]);
      setStartTime("09:00");
      setEndTime("10:00");
      onClose();
      onSave?.();
    } catch (err) {
      setError(
        `Failed to save: ${err instanceof Error ? err.message : String(err)}`
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2 className="modal-title">Add to Calendar</h2>
          <button className="modal-close-btn" onClick={onClose}>
            ✕
          </button>
        </div>

        <div className="modal-body">
          <div className="form-group">
            <label className="form-label">Title</label>
            <input
              type="text"
              className="form-input"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Focus session title"
            />
          </div>

          <div className="form-group">
            <label className="form-label">Date</label>
            <input
              type="date"
              className="form-input"
              value={date}
              onChange={(e) => setDate(e.target.value)}
            />
          </div>

          <div className="form-row">
            <div className="form-group">
              <label className="form-label">Start Time</label>
              <input
                type="time"
                className="form-input"
                value={startTime}
                onChange={(e) => setStartTime(e.target.value)}
              />
            </div>
            <div className="form-group">
              <label className="form-label">End Time</label>
              <input
                type="time"
                className="form-input"
                value={endTime}
                onChange={(e) => setEndTime(e.target.value)}
              />
            </div>
          </div>

          {error && (
            <div className="form-error" style={{ color: "var(--color-error, #d97706)" }}>
              {error}
            </div>
          )}
        </div>

        <div className="modal-actions">
          <button className="btn btn-secondary" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? "Saving..." : "Save Focus Session"}
          </button>
        </div>
      </div>
    </div>
  );
}
