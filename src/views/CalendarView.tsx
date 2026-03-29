import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { FocusSession, Meeting } from "../types";

// ── Date helpers ──────────────────────────────────────────────────────────────

function getWeekDates(weekOffset = 0): Date[] {
  const now = new Date();
  const dow = now.getDay(); // 0 = Sun
  const monday = new Date(now);
  monday.setDate(now.getDate() - (dow === 0 ? 6 : dow - 1) + weekOffset * 7);
  return Array.from({ length: 7 }, (_, i) => {
    const d = new Date(monday);
    d.setDate(monday.getDate() + i);
    return d;
  });
}

function toDateStr(d: Date): string {
  return d.toISOString().split("T")[0];
}

function timeToMinutes(t: string): number {
  const [h, m] = t.split(":").map(Number);
  return (h ?? 0) * 60 + (m ?? 0);
}

const DAY_START = 7 * 60;  // 07:00
const DAY_END   = 22 * 60; // 22:00
const DAY_MINS  = DAY_END - DAY_START; // 900 minutes visible

const HOURS = Array.from({ length: 16 }, (_, i) => i + 7); // 7 → 22

// ── Component ─────────────────────────────────────────────────────────────────

export default function CalendarView() {
  const [weekOffset, setWeekOffset] = useState(0);
  const [focusSessions, setFocusSessions] = useState<FocusSession[]>([]);
  const [meetings, setMeetings]           = useState<Meeting[]>([]);
  const [showForm, setShowForm]           = useState(false);
  const [formDate, setFormDate]           = useState(toDateStr(new Date()));
  const [formTitle, setFormTitle]         = useState("");
  const [formStart, setFormStart]         = useState("09:00");
  const [formEnd, setFormEnd]             = useState("10:00");
  const [formNotes, setFormNotes]         = useState("");
  const [formError, setFormError]         = useState("");
  const [calendarError, setCalendarError] = useState("");
  const [deletingId, setDeletingId]       = useState<string | null>(null);

  const weekDates = getWeekDates(weekOffset);
  const startDate = toDateStr(weekDates[0]);
  const endDate   = toDateStr(weekDates[6]);

  const fetchData = async () => {
    try {
      const [fs, ms] = await Promise.all([
        invoke<FocusSession[]>("get_focus_sessions", { startDate, endDate }),
        invoke<Meeting[]>("get_meetings"),
      ]);
      setFocusSessions(fs);
      setMeetings(ms);
    } catch {}
  };

  useEffect(() => { fetchData(); }, [weekOffset]);

  // [start, end) overlap check.
  const rangesOverlap = (startA: number, endA: number, startB: number, endB: number): boolean =>
    startA < endB && endA > startB;

  const hasConflict = (date: string, start: string, end: string): boolean => {
    const startMin = timeToMinutes(start);
    const endMin = timeToMinutes(end);
    const daySessions = sessionsByDate[date] ?? [];
    const dayMeetings = meetingsByDate[date] ?? [];

    if (daySessions.some((s) => rangesOverlap(startMin, endMin, timeToMinutes(s.start_time), timeToMinutes(s.end_time)))) {
      return true;
    }

    return dayMeetings.some((m) => {
      const startDateTime = new Date(m.started_at);
      if (Number.isNaN(startDateTime.getTime())) return false;
      const meetingStart = startDateTime.getHours() * 60 + startDateTime.getMinutes();

      const meetingEnd = m.ended_at
        ? (() => {
            const endDateTime = new Date(m.ended_at!);
            if (Number.isNaN(endDateTime.getTime())) return meetingStart + 60;
            return endDateTime.getHours() * 60 + endDateTime.getMinutes();
          })()
        : meetingStart + 60;

      return rangesOverlap(startMin, endMin, meetingStart, meetingEnd);
    });
  };

  const addSession = async () => {
    setFormError("");
    setCalendarError("");

    if (!formTitle.trim()) {
      setFormError("Title is required.");
      return;
    }
    if (timeToMinutes(formEnd) <= timeToMinutes(formStart)) {
      setFormError("End time must be after start time.");
      return;
    }
    if (hasConflict(formDate, formStart, formEnd)) {
      setFormError("This time is already booked by a focus block or meeting.");
      return;
    }

    const created = await invoke<boolean>("create_focus_session", {
      title: formTitle.trim(),
      date: formDate,
      startTime: formStart,
      endTime: formEnd,
      notes: formNotes.trim() || null,
    });

    if (!created) {
      setFormError("Could not create focus block. Time may already be booked.");
      return;
    }

    setFormTitle("");
    setFormNotes("");
    setFormError("");
    setShowForm(false);
    fetchData();
  };

  const deleteSession = async (id: string) => {
    setCalendarError("");
    setDeletingId(id);
    try {
      const deleted = await invoke<boolean>("delete_focus_session", { id });
      if (!deleted) {
        setCalendarError("Could not delete the focus block. Please try again.");
        return;
      }
      fetchData();
    } catch {
      setCalendarError("Could not delete the focus block. Please try again.");
    } finally {
      setDeletingId(null);
    }
  };

  const weekLabel = (() => {
    const s = weekDates[0].toLocaleDateString("en-US", { month: "short", day: "numeric" });
    const e = weekDates[6].toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
    return `${s} – ${e}`;
  })();

  const today = toDateStr(new Date());

  // Map date → focus sessions
  const sessionsByDate: Record<string, FocusSession[]> = {};
  for (const s of focusSessions) {
    if (!sessionsByDate[s.date]) sessionsByDate[s.date] = [];
    sessionsByDate[s.date].push(s);
  }

  // Map date → meetings (keyed by started_at date portion)
  const meetingsByDate: Record<string, Meeting[]> = {};
  for (const m of meetings) {
    const d = m.started_at.split("T")[0];
    if (d && d >= startDate && d <= endDate) {
      if (!meetingsByDate[d]) meetingsByDate[d] = [];
      meetingsByDate[d].push(m);
    }
  }

  return (
    <div className="view-calendar">
      {/* ── Header ─────────────────────────────────────── */}
      <div className="view-header">
        <div>
          <h1 className="view-title">Calendar</h1>
          <p className="view-subtitle">{weekLabel}</p>
        </div>
        <div className="header-actions">
          <button className="cal-nav-btn" onClick={() => setWeekOffset((n) => n - 1)}>← Prev</button>
          <button className="cal-nav-btn" onClick={() => setWeekOffset(0)} disabled={weekOffset === 0}>Today</button>
          <button className="cal-nav-btn" onClick={() => setWeekOffset((n) => n + 1)}>Next →</button>
          <button className="action-btn" onClick={() => setShowForm((s) => !s)}>
            + Focus Block
          </button>
        </div>
      </div>

      {/* ── Add session form ────────────────────────────── */}
      {showForm && (
        <div className="card cal-form-card">
          <div className="card-label">New Focus Block</div>
          <div className="cal-form-row">
            <input
              className="qa-input"
              placeholder="Title (e.g. Deep work — assistant app)"
              value={formTitle}
              onChange={(e) => setFormTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addSession()}
            />
            <input
              className="qa-input qa-small"
              type="date"
              value={formDate}
              onChange={(e) => setFormDate(e.target.value)}
            />
            <input
              className="qa-input qa-small"
              type="time"
              value={formStart}
              onChange={(e) => setFormStart(e.target.value)}
            />
            <input
              className="qa-input qa-small"
              type="time"
              value={formEnd}
              onChange={(e) => setFormEnd(e.target.value)}
            />
            <input
              className="qa-input"
              placeholder="Notes (optional)"
              value={formNotes}
              onChange={(e) => setFormNotes(e.target.value)}
            />
            <button className="action-btn" onClick={addSession}>Save</button>
            <button
              className="cal-nav-btn"
              onClick={() => {
                setShowForm(false);
                setFormError("");
              }}
            >
              Cancel
            </button>
          </div>
          {formError && <div className="cal-error-msg">{formError}</div>}
        </div>
      )}

      {calendarError && <div className="cal-error-msg">{calendarError}</div>}

      {/* ── Week grid ───────────────────────────────────── */}
      <div className="cal-grid-wrap card">
        {/* Time labels column */}
        <div className="cal-time-col">
          <div className="cal-day-header" /> {/* spacer */}
          <div className="cal-time-rows">
            {HOURS.map((h) => (
              <div key={h} className="cal-hour-label">
                {h === 12 ? "12 PM" : h < 12 ? `${h} AM` : `${h - 12} PM`}
              </div>
            ))}
          </div>
        </div>

        {/* Day columns */}
        {weekDates.map((date) => {
          const ds = toDateStr(date);
          const isToday = ds === today;
          const dayLabel = date.toLocaleDateString("en-US", { weekday: "short" });
          const dayNum   = date.getDate();
          const daySessions = sessionsByDate[ds] ?? [];
          const dayMeetings = meetingsByDate[ds] ?? [];

          return (
            <div key={ds} className={`cal-day-col${isToday ? " cal-today" : ""}`}>
              <div
                className="cal-day-header"
                onClick={() => {
                  setFormDate(ds);
                  setShowForm(true);
                }}
                title="Add focus block"
              >
                <span className="cal-day-name">{dayLabel}</span>
                <span className={`cal-day-num${isToday ? " cal-today-num" : ""}`}>{dayNum}</span>
              </div>

              <div className="cal-event-area">
                {/* Hour grid lines */}
                {HOURS.map((h) => (
                  <div key={h} className="cal-hour-line" />
                ))}

                {/* Focus session blocks */}
                {daySessions.map((s) => {
                  const startMin = timeToMinutes(s.start_time) - DAY_START;
                  const endMin   = timeToMinutes(s.end_time)   - DAY_START;
                  const top    = Math.max(0, (startMin / DAY_MINS) * 100);
                  const height = Math.max(2, ((endMin - startMin) / DAY_MINS) * 100);
                  return (
                    <div
                      key={s.id}
                      className="cal-event cal-focus"
                      style={{ top: `${top}%`, height: `${height}%` }}
                      title={`${s.title}\n${s.start_time}–${s.end_time}${s.notes ? `\n${s.notes}` : ""}`}
                    >
                      <button
                        className="cal-event-delete-btn"
                        onClick={(e) => {
                          e.stopPropagation();
                          if (deletingId !== s.id) {
                            deleteSession(s.id);
                          }
                        }}
                        title="Delete focus block"
                        aria-label={`Delete focus block ${s.title}`}
                      >
                        {deletingId === s.id ? "..." : "×"}
                      </button>
                      <span className="cal-event-title">{s.title}</span>
                      <span className="cal-event-time">{s.start_time}–{s.end_time}</span>
                    </div>
                  );
                })}

                {/* Meeting blocks — use started_at time */}
                {dayMeetings.map((m) => {
                  const t = new Date(m.started_at);
                  const startMin = t.getHours() * 60 + t.getMinutes() - DAY_START;
                  const endMin   = m.ended_at
                    ? (() => { const e = new Date(m.ended_at!); return e.getHours() * 60 + e.getMinutes() - DAY_START; })()
                    : startMin + 60; // default 1h if still live
                  const top    = Math.max(0, (startMin / DAY_MINS) * 100);
                  const height = Math.max(2, ((endMin - startMin) / DAY_MINS) * 100);
                  return (
                    <div
                      key={m.id}
                      className="cal-event cal-meeting"
                      style={{ top: `${top}%`, height: `${height}%` }}
                      title={`${m.title}${m.person ? ` — ${m.person}` : ""}`}
                    >
                      <span className="cal-event-title">{m.title}</span>
                      {m.person && <span className="cal-event-time">{m.person}</span>}
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      {/* ── Legend ──────────────────────────────────────── */}
      <div className="cal-legend">
        <span className="cal-legend-item cal-legend-focus">Focus block</span>
        <span className="cal-legend-item cal-legend-meeting">Meeting</span>
      </div>
    </div>
  );
}
