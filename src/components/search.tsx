// Memory search UI, shared by the Search view and the menu-bar popover.
// Ranking happens in the backend (EmbeddingGemma 2 + keyword boost); this
// only debounces the query and renders hits.

import { useEffect, useRef, useState, type ReactNode } from "react";
import * as api from "../lib/api";
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
  placeholder = "Search what you've said — by meaning…",
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

function fmtDay(day: string): string {
  const d = new Date(`${day}T12:00:00`);
  const today = new Date();
  const diff = Math.round((today.setHours(12, 0, 0, 0) - d.getTime()) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

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
  /** Optional extra action per hit (e.g. "show in mind map"). */
  onOpen?: (hit: SearchHit) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const [full, setFull] = useState<Thought | null>(null);

  const toggle = async (h: SearchHit) => {
    if (open === h.thoughtId) {
      setOpen(null);
      return;
    }
    setOpen(h.thoughtId);
    setFull(null);
    setFull(await api.getThought(h.thoughtId).catch(() => null));
  };

  if (error) return <p className="mt-3 text-xs text-danger">{error}</p>;
  if (query.trim().length >= 2 && hits.length === 0)
    return <p className="mt-3 text-xs text-ink-3">Nothing you've said comes close to that yet.</p>;

  const weak = hits.length > 0 && hits[0].weak;
  return (
    <>
    {weak && (
      <p className="mt-3 text-xs text-ink-3">No clear match — these are the closest things you've said:</p>
    )}
    <ul className={`mt-3 space-y-2 ${compact ? "" : "pb-6"} ${weak ? "opacity-70" : ""}`}>
      {hits.map((h) => (
        <li key={h.thoughtId} className="rounded-xl border border-line bg-surface">
          <button onClick={() => void toggle(h)} className="block w-full px-3 py-2.5 text-left">
            <div className="mb-1 flex items-center gap-2 text-[11px] text-ink-3">
              <span>{fmtDay(h.day)}</span>
              {h.strong && <span className="rounded-full bg-ok/12 px-1.5 py-px text-ok">strong match</span>}
            </div>
            <p className={`text-[13px] leading-relaxed text-ink ${compact && open !== h.thoughtId ? "line-clamp-2" : ""}`}>
              {highlight(h.snippet, h.matched)}
            </p>
            {!compact && h.summary && <p className="mt-1 text-xs text-ink-3">{h.summary}</p>}
          </button>
          {open === h.thoughtId && (
            <div className="border-t border-line px-3 py-2.5">
              {full ? (
                <p className="max-h-56 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-ink-2">
                  {highlight(full.text, h.matched)}
                </p>
              ) : (
                <p className="text-xs text-ink-3">Loading…</p>
              )}
              {onOpen && (
                <button onClick={() => onOpen(h)} className="mt-2 text-xs font-medium text-accent hover:underline">
                  Show in mind map →
                </button>
              )}
            </div>
          )}
        </li>
      ))}
    </ul>
    </>
  );
}
