// Today: one screen for the day — a friction insight, the checklist,
// focus blocks, and what the calendar already owns.

import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "../lib/api";
import type {
  CalendarEvent,
  FocusSession,
  Task,
  TodayBrief,
} from "../lib/types";

interface Props {
  refreshKey: number; // bumped on capture-complete / task events
}

export default function TodayView({ refreshKey }: Props) {
  const [brief, setBrief] = useState<TodayBrief | null>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [permissionHint, setPermissionHint] = useState<string | null>(null);
  const [focus, setFocus] = useState<FocusSession[]>([]);
  const [activeFocus, setActiveFocus] = useState<FocusSession | null>(null);
  const [now, setNow] = useState(Date.now());

  const loadTasks = useCallback(() => {
    void api
      .getTasks(false)
      .then(setTasks)
      .catch(() => setTasks([]));
  }, []);

  const loadFocus = useCallback(() => {
    void api
      .getFocusToday()
      .then((sessions) => {
        setFocus(sessions);
        setActiveFocus(sessions.find((s) => s.endedAt === null) ?? null);
      })
      .catch(() => setFocus([]));
  }, []);

  // Full refresh: calendar first so the brief sees today's events. The brief
  // itself is cached per day by the backend, so opening Today is instant;
  // task and focus events refresh just their own card.
  const load = useCallback(() => {
    loadTasks();
    loadFocus();
    void api
      .syncCalendar()
      .then((s) => {
        setEvents(s.events);
        setPermissionHint(s.permissionHint);
      })
      .catch(() => setEvents([]))
      .finally(() => {
        void api
          .getTodayBrief()
          .then(setBrief)
          .catch(() => setBrief(null));
      });
  }, [loadTasks, loadFocus]);

  useEffect(load, [load, refreshKey]);

  // 1s clock for the focus countdown.
  const clockRef = useRef<number | null>(null);
  useEffect(() => {
    if (!activeFocus) return;
    clockRef.current = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      if (clockRef.current !== null) window.clearInterval(clockRef.current);
    };
  }, [activeFocus]);

  // Live refresh from backend events. (No `calendar-synced` listener: this
  // view's own sync emits it, which used to re-trigger `load` in a loop.)
  useEffect(() => {
    const un1 = api.on<unknown>("tasks-updated", loadTasks);
    const un2 = api.on<unknown>("focus-updated", loadFocus);
    // A fresh insight is generated in the background after each capture.
    const un3 = api.on<TodayBrief>("today-brief", setBrief);
    return () => {
      void un1.then((f) => f());
      void un2.then((f) => f());
      void un3.then((f) => f());
    };
  }, [loadTasks, loadFocus]);

  const startFocus = async () => {
    try {
      const started = await api.startFocus(25);
      const session: FocusSession = {
        id: started.id,
        title: null,
        day: new Date().toISOString().slice(0, 10),
        startedAt: started.startedAt,
        endedAt: null,
        minutes: started.minutes,
      };
      setActiveFocus(session);
      setFocus((prev) => [session, ...prev.filter((s) => s.endedAt !== null)]);
    } catch (e) {
      alert(String(e));
    }
  };

  const finishFocus = async () => {
    if (!activeFocus) return;
    const elapsed = Math.round(
      (Date.now() - new Date(activeFocus.startedAt).getTime()) / 60000,
    );
    try {
      await api.finishFocus(activeFocus.id, Math.max(elapsed, 0));
      setActiveFocus(null); // `focus-updated` refreshes the list
    } catch (e) {
      alert(String(e));
    }
  };

  const toggleTask = async (t: Task) => {
    await api.updateTask(t.id, null, !t.done);
    setTasks((prev) => prev.filter((x) => x.id !== t.id || !t.done));
  };

  const removeTask = async (t: Task) => {
    await api.deleteTask(t.id);
    setTasks((prev) => prev.filter((x) => x.id !== t.id));
  };

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mx-auto grid max-w-4xl grid-cols-1 gap-4 lg:grid-cols-2">
        {/* brief + focus, left column */}
        <div className="space-y-4">
          {brief && (brief.friction || brief.suggestion) && (
            <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
              <h3 className="mb-2 flex items-center gap-2 text-sm font-bold">
                <span className="text-base">✦</span> Your friction
              </h3>
              {brief.friction && (
                <p className="text-[13px] leading-relaxed text-ink">{brief.friction}</p>
              )}
              {brief.suggestion && (
                <p className="mt-2 rounded-lg bg-accent/8 px-3 py-2 text-xs text-ink-2">
                  <span className="font-semibold text-accent">Next step: </span>
                  {brief.suggestion}
                </p>
              )}
            </section>
          )}

          <FocusCard
            active={activeFocus}
            now={now}
            sessions={focus}
            onStart={startFocus}
            onFinish={finishFocus}
          />
        </div>

        {/* checklist + calendar, right column */}
        <div className="space-y-4">
          <TaskCard tasks={tasks} onToggle={toggleTask} onRemove={removeTask} onRefresh={loadTasks} />
          <CalendarCard events={events} hint={permissionHint} onResync={load} />
        </div>
      </div>
    </div>
  );
}

// ── focus ───────────────────────────────────────────────────────────────────

function FocusCard({
  active,
  now,
  sessions,
  onStart,
  onFinish,
}: {
  active: FocusSession | null;
  now: number;
  sessions: FocusSession[];
  onStart: () => void;
  onFinish: () => void;
}) {
  const total = sessions.reduce((s, x) => s + x.minutes, 0);

  let remaining = 0;
  let totalSecs = 25 * 60;
  if (active) {
    const started = new Date(active.startedAt).getTime();
    totalSecs = Math.max(60, Math.round((now - started) / 1000));
    remaining = active.minutes * 60 - Math.round((now - started) / 1000);
  }
  const pct = active ? Math.max(0, Math.min(100, (remaining / (active.minutes * 60)) * 100)) : 0;

  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
      <div className="mb-3 flex items-baseline justify-between">
        <h3 className="text-sm font-bold">Focus</h3>
        <span className="text-[11px] text-ink-3">
          {total > 0 ? `${Math.round(total)} min logged today` : "nothing logged yet"}
        </span>
      </div>

      {active ? (
        <div className="flex items-center gap-4">
          <div className="relative h-16 w-16 shrink-0">
            <svg viewBox="0 0 36 36" className="h-16 w-16 -rotate-90">
              <circle cx="18" cy="18" r="15.9" fill="none" stroke="var(--line)" strokeWidth="3" />
              <circle
                cx="18"
                cy="18"
                r="15.9"
                fill="none"
                stroke="var(--accent)"
                strokeWidth="3"
                strokeDasharray={`${pct} 100`}
                strokeLinecap="round"
              />
            </svg>
            <span className="absolute inset-0 flex items-center justify-center text-[11px] font-bold tabular-nums">
              {fmtCountdown(Math.max(0, remaining))}
            </span>
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-xs font-semibold">In a focus block</p>
            <p className="mt-0.5 text-[11px] text-ink-3">
              {Math.round(totalSecs / 60)} min in · {active.minutes}-min block
            </p>
          </div>
          <button
            onClick={onFinish}
            className="rounded-xl bg-ok px-4 py-2 text-xs font-semibold text-white hover:opacity-90"
          >
            Finish
          </button>
        </div>
      ) : (
        <button
          onClick={onStart}
          className="w-full rounded-xl border border-line bg-surface-2 py-3 text-sm font-semibold text-ink-2 transition-colors hover:border-accent hover:text-accent"
        >
          ▶ Start a 25-minute focus block
        </button>
      )}
    </section>
  );
}

function fmtCountdown(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

// ── tasks ───────────────────────────────────────────────────────────────────

function TaskCard({
  tasks,
  onToggle,
  onRemove,
  onRefresh,
}: {
  tasks: Task[];
  onToggle: (t: Task) => void;
  onRemove: (t: Task) => void;
  onRefresh: () => void;
}) {
  const [title, setTitle] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");

  const add = async () => {
    const t = title.trim();
    if (t.length < 2) return;
    try {
      await api.addTask(t);
      setTitle("");
      onRefresh();
    } catch (e) {
      alert(String(e));
    }
  };

  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
      <div className="mb-3 flex items-baseline justify-between">
        <h3 className="text-sm font-bold">Checklist</h3>
        <span className="text-[11px] text-ink-3">{tasks.length} open</span>
      </div>

      <div className="mb-2 flex gap-2">
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void add()}
          placeholder="Add a task…"
          className="min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs outline-none placeholder:text-ink-3 focus:border-accent"
        />
        <button
          onClick={() => void add()}
          disabled={title.trim().length < 2}
          className="rounded-lg bg-accent px-3 text-xs font-semibold text-white disabled:opacity-40"
        >
          Add
        </button>
      </div>

      <ul className="space-y-1.5">
        {tasks.map((t) => (
          <li key={t.id} className="group flex items-center gap-2.5">
            <button
              onClick={() => void onToggle(t)}
              className={`flex h-4.5 w-4.5 shrink-0 items-center justify-center rounded-md border text-[10px] transition-colors ${
                t.done
                  ? "border-ok bg-ok text-white"
                  : "border-line-strong text-transparent hover:border-accent"
              }`}
              title={t.done ? "Mark as open" : "Mark done"}
            >
              ✓
            </button>
            {editing === t.id ? (
              <input
                autoFocus
                value={editTitle}
                onChange={(e) => setEditTitle(e.target.value)}
                onBlur={() => {
                  if (editTitle.trim().length >= 2)
                    void api.updateTask(t.id, editTitle.trim(), null);
                  setEditing(null);
                  onRefresh();
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                  if (e.key === "Escape") setEditing(null);
                }}
                className="min-w-0 flex-1 rounded-md border border-accent bg-surface-2 px-2 py-0.5 text-xs outline-none"
              />
            ) : (
              <button
                onDoubleClick={() => {
                  setEditing(t.id);
                  setEditTitle(t.title);
                }}
                className={`min-w-0 flex-1 truncate text-left text-[13px] ${
                  t.done ? "text-ink-3 line-through" : "text-ink-2"
                }`}
                title={`${t.title} (double-click to rename)`}
              >
                {t.title}
              </button>
            )}
            {t.project && (
              <span className="hidden rounded-full bg-surface-2 px-2 py-0.5 text-[10px] text-ink-3 lg:inline">
                {t.project}
              </span>
            )}
            <button
              onClick={() => void onRemove(t)}
              className="text-ink-3 opacity-0 transition-opacity hover:text-danger group-hover:opacity-100"
              title="Delete"
            >
              ✕
            </button>
          </li>
        ))}
        {tasks.length === 0 && (
          <li className="py-2 text-center text-xs text-ink-3">
            Nothing open — either a good day, or a brain-dump away from one.
          </li>
        )}
      </ul>
    </section>
  );
}

// ── calendar ────────────────────────────────────────────────────────────────

function eventTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 16);
  return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function CalendarCard({
  events,
  hint,
  onResync,
}: {
  events: CalendarEvent[];
  hint: string | null;
  onResync: () => void;
}) {
  const scheduled = events.reduce((sum, e) => {
    const s = new Date(e.start).getTime();
    const en = new Date(e.end).getTime();
    return Number.isNaN(s) || Number.isNaN(en) ? sum : sum + (en - s) / 3600000;
  }, 0);

  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm backdrop-blur-xl fade-up">
      <div className="mb-3 flex items-baseline justify-between">
        <h3 className="text-sm font-bold">Calendar</h3>
        <button
          onClick={onResync}
          className="text-[11px] font-medium text-ink-3 hover:text-accent"
          title="Re-sync Apple Calendar"
        >
          {scheduled > 0 ? `≈ ${Math.round(scheduled)} h scheduled` : "re-sync"}
        </button>
      </div>
      {hint && (
        <p className="mb-2 rounded-lg bg-warn/10 px-3 py-2 text-[11px] leading-relaxed text-ink-2">
          {hint}
        </p>
      )}
      <ul className="space-y-1.5">
        {events.map((e) => (
          <li key={e.id} className="flex items-baseline gap-2.5">
            <span className="w-14 shrink-0 text-[11px] tabular-nums text-ink-3">
              {eventTime(e.start)}
            </span>
            <span className="min-w-0 flex-1 truncate text-[13px] text-ink-2">{e.title}</span>
            <span className="shrink-0 text-[10px] text-ink-3">{e.calendar}</span>
          </li>
        ))}
        {events.length === 0 && !hint && (
          <li className="py-2 text-center text-xs text-ink-3">
            Nothing on the calendar today.
          </li>
        )}
      </ul>
    </section>
  );
}
