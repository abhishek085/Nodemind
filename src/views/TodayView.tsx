// Today: your checklist, grouped by the thought it came from, next to a
// timeline of what you captured today and what the calendar holds.
//
// Everything here is factual — no AI commentary. Tasks arrive already
// de-duplicated (engine/tasks.rs); this view only shows and edits them.

import { useCallback, useEffect, useState } from "react";
import * as api from "../lib/api";
import { LivePartial } from "../components/capture";
import { FogList, FogTag } from "../components/fog";
import type { CalendarEvent, Today, TodayTask, TodayThought } from "../lib/types";

interface Props {
  refreshKey: number; // bumped on capture-complete
}

function clock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export default function TodayView({ refreshKey }: Props) {
  const [today, setToday] = useState<Today | null>(null);
  const [events, setEvents] = useState<CalendarEvent[]>([]);
  const [calendarHint, setCalendarHint] = useState<string | null>(null);

  const load = useCallback(() => {
    void api.getToday().then(setToday).catch(() => undefined);
  }, []);

  const loadCalendar = useCallback(() => {
    void api
      .syncCalendar()
      .then((s) => {
        setEvents(s.events);
        setCalendarHint(s.permissionHint);
      })
      .catch(() => setEvents([]));
  }, []);

  useEffect(load, [load, refreshKey]);
  useEffect(loadCalendar, [loadCalendar, refreshKey]);

  useEffect(() => {
    const subs = [api.on<unknown>("tasks-updated", load), api.on<unknown>("thoughts-updated", load)];
    return () => {
      for (const un of subs) void un.then((f) => f());
    };
  }, [load]);

  const date = new Date().toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mx-auto max-w-5xl">
        <header className="mb-5">
          <h2 className="text-xl font-bold">{date}</h2>
          {today && (
            <p className="mt-0.5 text-xs text-ink-3">
              {plural(today.tasks.length, "open task")} · {plural(today.thoughts.length, "thought")} today
              {events.length > 0 && ` · ${plural(events.length, "event")}`}
            </p>
          )}
        </header>

        <div className="grid grid-cols-1 gap-4 lg:grid-cols-5">
          <div className="lg:col-span-3">
            <Checklist tasks={today?.tasks ?? []} onChanged={load} />
          </div>
          <div className="space-y-4 lg:col-span-2">
            <StuckCard thoughts={today?.thoughts ?? []} />
            <Thoughts thoughts={today?.thoughts ?? []} onChanged={load} />
            <CalendarCard events={events} hint={calendarHint} onResync={loadCalendar} />
          </div>
        </div>
      </div>
    </div>
  );
}

// ── checklist ───────────────────────────────────────────────────────────────

interface Group {
  key: string;
  label: string;
  tasks: TodayTask[];
}

/** Tasks grouped by the capture they came from, newest capture first. */
function groupTasks(tasks: TodayTask[]): Group[] {
  const groups = new Map<string, Group>();
  for (const t of tasks) {
    const key = t.source?.thoughtId ?? "typed";
    if (!groups.has(key)) {
      const s = t.source;
      const label = !s
        ? "Added by you"
        : s.kind === "meeting"
          ? `From meeting · ${s.title}`
          : `From your ${clock(s.startedAt)} note${isToday(s.startedAt) ? "" : ` (${shortDay(s.startedAt)})`}`;
      groups.set(key, { key, label, tasks: [] });
    }
    groups.get(key)!.tasks.push(t);
  }
  return [...groups.values()];
}

function isToday(iso: string): boolean {
  return new Date(iso).toDateString() === new Date().toDateString();
}

function shortDay(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function Checklist({ tasks, onChanged }: { tasks: TodayTask[]; onChanged: () => void }) {
  const [title, setTitle] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [leaving, setLeaving] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");

  const add = async () => {
    const t = title.trim();
    if (t.length < 2) return;
    const added = await api.addTask(t).catch(() => null);
    setTitle("");
    setNote(added ? null : "Already on your list");
    onChanged();
  };

  // Tick → strike through briefly → gone.
  const complete = async (id: string) => {
    setLeaving((s) => new Set(s).add(id));
    await api.updateTask(id, null, true).catch(() => undefined);
    window.setTimeout(onChanged, 450);
  };

  const remove = async (id: string) => {
    await api.deleteTask(id).catch(() => undefined);
    onChanged();
  };

  const groups = groupTasks(tasks);

  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm fade-up">
      <h3 className="mb-3 text-sm font-bold">Checklist</h3>
      <input
        value={title}
        onChange={(e) => {
          setTitle(e.target.value);
          setNote(null);
        }}
        onKeyDown={(e) => e.key === "Enter" && void add()}
        placeholder="Add a task and press Enter"
        className="w-full rounded-lg border border-line bg-surface-2 px-3 py-2 text-[13px] outline-none placeholder:text-ink-3 focus:border-accent"
      />
      {note && <p className="mt-1.5 text-[11px] text-ink-3">{note}</p>}

      {groups.length === 0 ? (
        <p className="py-8 text-center text-xs text-ink-3">
          Nothing on your list. Hold <kbd className="font-mono">⌥⇧Space</kbd> and talk it out.
        </p>
      ) : (
        <div className="mt-4 space-y-4">
          {groups.map((g) => (
            <div key={g.key}>
              <p className="mb-1.5 truncate text-[11px] font-medium uppercase tracking-wide text-ink-3">{g.label}</p>
              <ul className="space-y-0.5">
                {g.tasks.map((t) => {
                  const done = leaving.has(t.id);
                  return (
                    <li key={t.id} className="group flex items-center gap-3 rounded-lg px-1 py-1.5 hover:bg-surface-2">
                      <button
                        onClick={() => void complete(t.id)}
                        className={`flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border text-[10px] transition-colors ${
                          done ? "border-ok bg-ok text-white" : "border-line-strong text-transparent hover:border-ok"
                        }`}
                        title="Done"
                      >
                        ✓
                      </button>
                      {editing === t.id ? (
                        <input
                          autoFocus
                          value={editTitle}
                          onChange={(e) => setEditTitle(e.target.value)}
                          onBlur={() => {
                            if (editTitle.trim().length >= 2 && editTitle.trim() !== t.title)
                              void api.updateTask(t.id, editTitle.trim(), null).then(onChanged);
                            setEditing(null);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                            if (e.key === "Escape") setEditing(null);
                          }}
                          className="min-w-0 flex-1 rounded-md border border-accent bg-surface px-2 py-0.5 text-[13px] outline-none"
                        />
                      ) : (
                        <span
                          onDoubleClick={() => {
                            setEditing(t.id);
                            setEditTitle(t.title);
                          }}
                          className={`min-w-0 flex-1 text-[13px] transition-colors ${done ? "text-ink-3 line-through" : "text-ink"}`}
                          title="Double-click to edit"
                        >
                          {t.title}
                        </span>
                      )}
                      <button
                        onClick={() => void remove(t.id)}
                        className="text-xs text-ink-3 opacity-0 transition-opacity hover:text-danger group-hover:opacity-100"
                        title="Delete"
                      >
                        ✕
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

// ── where you're stuck ──────────────────────────────────────────────────────

/** Today's fog, newest first, each with the question that unsticks it. */
function StuckCard({ thoughts }: { thoughts: TodayThought[] }) {
  const items = thoughts.flatMap((t) => (t.fog ?? []).map((f) => ({ f, at: t.startedAt })));
  if (items.length === 0) return null;
  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm fade-up">
      <h3 className="mb-1 text-sm font-bold">Where you're stuck</h3>
      <p className="mb-3 text-[11px] text-ink-3">Patterns in how you talked today — answer the question to get moving.</p>
      <FogList signals={items.map((i) => i.f)} when={(i) => clock(items[i].at)} />
    </section>
  );
}

// ── today's thoughts ────────────────────────────────────────────────────────

function Thoughts({ thoughts, onChanged }: { thoughts: TodayThought[]; onChanged: () => void }) {
  const [open, setOpen] = useState<string | null>(null);

  const remove = async (t: TodayThought) => {
    if (!window.confirm(`Delete this ${t.kind === "meeting" ? "meeting" : "note"} and its ${plural(t.taskCount, "task")}?`)) return;
    await api.deleteThought(t.id).catch(() => undefined);
    setOpen(null);
    onChanged();
  };

  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm fade-up">
      <h3 className="mb-3 text-sm font-bold">Today's thoughts</h3>
      {thoughts.length === 0 ? (
        <p className="py-4 text-center text-xs text-ink-3">Nothing captured yet today.</p>
      ) : (
        <ol className="relative space-y-1 border-l border-line pl-4">
          {thoughts.map((t) => (
            <li key={t.id} className="relative">
              <span
                className={`absolute -left-[21px] top-3 h-2.5 w-2.5 rounded-full border-2 border-surface ${
                  t.kind === "meeting" ? "bg-accent-2" : "bg-accent"
                }`}
              />
              <button
                onClick={() => setOpen(open === t.id ? null : t.id)}
                className="block w-full rounded-lg px-2 py-2 text-left hover:bg-surface-2"
              >
                <p className="text-[11px] text-ink-3">
                  {clock(t.startedAt)} · {t.kind === "meeting" ? "Meeting" : "Note"} ·{" "}
                  {Math.max(1, Math.round(t.durationSecs / 60))} min
                  {t.taskCount > 0 && ` · ${plural(t.taskCount, "task")}`}
                </p>
                <p className="mt-0.5 text-[13px] font-medium leading-snug text-ink">{t.title}</p>
                {(t.fog ?? []).length > 0 && (
                  <span className="mt-1 inline-flex gap-1">
                    {(t.fog ?? []).map((f, i) => (
                      <FogTag key={i} tag={f.tag} />
                    ))}
                  </span>
                )}
              </button>
              {open === t.id && (
                <div className="mb-2 space-y-2 px-2 pb-2">
                  {t.minutes ? (
                    <>
                      {t.minutes.summary && <p className="text-xs leading-relaxed text-ink-2">{t.minutes.summary}</p>}
                      {t.minutes.decisions.length > 0 && (
                        <div>
                          <p className="text-[11px] font-semibold text-ink-3">Decisions</p>
                          <ul className="list-disc pl-4 text-xs text-ink-2">
                            {t.minutes.decisions.map((d, i) => (
                              <li key={i}>{d}</li>
                            ))}
                          </ul>
                        </div>
                      )}
                      {t.minutes.actionItems.length > 0 && (
                        <div>
                          <p className="text-[11px] font-semibold text-ink-3">Action items</p>
                          <ul className="text-xs text-ink-2">
                            {t.minutes.actionItems.map((a, i) => (
                              <li key={i}>
                                <span className="font-medium text-ink">{a.owner || "You"}:</span> {a.task}
                                {a.due && <span className="text-ink-3"> · {a.due}</span>}
                              </li>
                            ))}
                          </ul>
                        </div>
                      )}
                    </>
                  ) : null}
                  <details className="text-xs text-ink-3" open={!t.minutes}>
                    <summary className="cursor-pointer">Transcript</summary>
                    <LivePartial text={t.text} className="mt-1 max-h-48" />
                  </details>
                  <button onClick={() => void remove(t)} className="text-[11px] text-ink-3 hover:text-danger">
                    Delete
                  </button>
                </div>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

// ── calendar ────────────────────────────────────────────────────────────────

function CalendarCard({
  events,
  hint,
  onResync,
}: {
  events: CalendarEvent[];
  hint: string | null;
  onResync: () => void;
}) {
  return (
    <section className="rounded-2xl bg-surface p-5 shadow-sm fade-up">
      <div className="mb-3 flex items-baseline justify-between">
        <h3 className="text-sm font-bold">Calendar</h3>
        <button onClick={onResync} className="text-[11px] font-medium text-ink-3 hover:text-accent">
          Refresh
        </button>
      </div>
      {hint && <p className="mb-2 rounded-lg bg-warn/10 px-3 py-2 text-[11px] leading-relaxed text-ink-2">{hint}</p>}
      <ul className="space-y-1.5">
        {events.map((e) => (
          <li key={e.id} className="flex items-baseline gap-2.5">
            <span className="w-16 shrink-0 text-[11px] tabular-nums text-ink-3">{clock(e.start)}</span>
            <span className="min-w-0 flex-1 truncate text-[13px] text-ink-2">{e.title}</span>
          </li>
        ))}
        {events.length === 0 && !hint && <li className="py-2 text-center text-xs text-ink-3">Nothing on the calendar today.</li>}
      </ul>
    </section>
  );
}
