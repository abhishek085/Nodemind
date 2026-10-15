// Memory search UI, shared by the Search view and the menu-bar popover.
// Ranking happens in the backend (EmbeddingGemma 2 + keyword boost); this
// only debounces the query and renders hits.

import { useEffect, useRef, useState, type ReactNode } from "react";
import * as api from "../lib/api";
import { LivePartial } from "./capture";
import { KindBadge } from "./items";
import type { SearchHit, Thought } from "../lib/types";

/** Search as you type: a query embeds in ~50 ms, so 200 ms debounce is plenty. */
export function useSearch(k = 12) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setHits([]);
      setError(null);
      return;
    }
    const id = ++seq.current;
    const t = window.setTimeout(() => {
      setBusy(true);
      api
        .searchMemory(q, k)
        .then((h) => {
          if (id === seq.current) {
            setHits(h);
            setError(null);
          }
        })
        .catch((e) => id === seq.current && setError(String(e)))
        .finally(() => id === seq.current && setBusy(false));
    }, 200);
    return () => window.clearTimeout(t);
  }, [query, k]);

  return { query, setQuery, hits, busy, error };
}

export function SearchInput({
  value,
  onChange,
  busy,
  autoFocus,
  placeholder = "Search ideas, questions, decisions, tasks…",
}: {
  value: string;
  onChange: (v: string) => void;
  busy?: boolean;
  autoFocus?: boolean;
  placeholder?: string;
}) {
  return (
    <div className="relative">
      <svg viewBox="0 0 24 24" className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-3" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="11" cy="11" r="7" />
        <path d="m20 20-3.5-3.5" strokeLinecap="round" />
      </svg>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoFocus={autoFocus}
        placeholder={placeholder}
        className="w-full rounded-xl border border-line bg-surface py-2 pl-9 pr-8 text-sm outline-none placeholder:text-ink-3 focus:border-accent"
      />
      {busy && (
        <span className="absolute right-3 top-1/2 h-3 w-3 -translate-y-1/2 animate-spin rounded-full border border-line border-t-accent" />
      )}
    </div>
  );
}

/** Bold the words the query matched verbatim. */
function highlight(text: string, words: string[]): ReactNode {
  if (words.length === 0) return text;
  const esc = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp(`(${esc.join("|")})`, "gi"));
  return parts.map((p, i) =>
    words.some((w) => w.toLowerCase() === p.toLowerCase()) ? (
      <mark key={i} className="rounded bg-accent/15 px-0.5 text-ink">
        {p}
      </mark>
    ) : (
      p
    ),
  );
}

function fmtWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  const days = Math.round((new Date(today.toDateString()).getTime() - new Date(d.toDateString()).getTime()) / 86_400_000);
  const day = days === 0 ? "Today" : days === 1 ? "Yesterday" : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  return `${day} · ${d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
}

/** Statements you made that match, each expandable to its recording. */
export function SearchResults({
  hits,
  query,
  error,
  compact = false,
  onOpen,
}: {
  hits: SearchHit[];
  query: string;
  error: string | null;
  compact?: boolean;
  /** Extra action for items (e.g. "show in mind map"); tasks aren't on the map. */
  onOpen?: (hit: SearchHit) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const [source, setSource] = useState<Thought | null>(null);

  const toggle = async (h: SearchHit) => {
    if (open === h.id) {
      setOpen(null);
      return;
    }
    setOpen(h.id);
    setSource(null);
    if (h.thoughtId) setSource(await api.getThought(h.thoughtId).catch(() => null));
  };

  if (error) return <p className="mt-3 text-xs text-danger">{error}</p>;
  if (query.trim().length >= 2 && hits.length === 0)
    return <p className="mt-3 text-xs text-ink-3">Nothing about that yet.</p>;

  return (
    <ul className={`mt-3 space-y-1.5 ${compact ? "" : "pb-6"}`}>
      {hits.map((h) => (
        <li key={h.id} className="rounded-xl border border-line bg-surface">
          <button onClick={() => void toggle(h)} className="block w-full px-3 py-2 text-left">
            <div className="mb-0.5 flex items-center gap-2">
              <KindBadge kind={h.kind} />
              <span className="text-[11px] text-ink-3">{fmtWhen(h.saidAt)}</span>
            </div>
            <p className={`text-[13px] leading-snug text-ink ${compact && open !== h.id ? "line-clamp-2" : ""}`}>
              {highlight(h.text, h.matched)}
            </p>
          </button>
          {open === h.id && (
            <div className="border-t border-line px-3 py-2">
              {h.thoughtId ? (
                source ? (
                  <details className="text-xs text-ink-3">
                    <summary className="cursor-pointer">What you said</summary>
                    <LivePartial text={source.text} className="mt-1 max-h-48" />
                  </details>
                ) : (
                  <p className="text-xs text-ink-3">Loading…</p>
                )
              ) : (
                <p className="text-xs text-ink-3">Typed by you.</p>
              )}
              {onOpen && (
                <button onClick={() => onOpen(h)} className="mt-1.5 text-xs font-medium text-accent hover:underline">
                  Show in mind map →
                </button>
              )}
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}
