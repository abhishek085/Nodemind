// Fog: where the thinking seemed stuck, and one question to get moving.
// Shared by the capture result, Today, and the mind map.

import type { FogSignal } from "../lib/types";

export const FOG_STYLE: Record<string, { label: string; color: string; hint: string }> = {
  overthinking: { label: "Overthinking", color: "#d97706", hint: "going round in circles" },
  unclarity: { label: "Unclear", color: "#64748b", hint: "no concrete next step" },
  abstraction: { label: "Abstract", color: "#7c5ce4", hint: "big idea, no first action" },
  overwhelmed: { label: "Overwhelmed", color: "#dc2626", hint: "too much at once" },
};

export function fogStyle(tag: string) {
  return FOG_STYLE[tag] ?? { label: tag, color: "#64748b", hint: "" };
}

export function FogTag({ tag }: { tag: string }) {
  const s = fogStyle(tag);
  return (
    <span
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold"
      style={{ background: `${s.color}1f`, color: s.color }}
      title={s.hint}
    >
      <span className="h-1.5 w-1.5 rounded-full" style={{ background: s.color }} />
      {s.label}
    </span>
  );
}

/** Each signal: the kind of fog, what it's about, and the unsticking question. */
export function FogList({ signals, when }: { signals: FogSignal[]; when?: (i: number) => string | undefined }) {
  return (
    <ul className="space-y-3">
      {signals.map((s, i) => (
        <li key={i}>
          <div className="flex flex-wrap items-center gap-2">
            <FogTag tag={s.tag} />
            <span className="text-xs text-ink-2">{s.context}</span>
            {when?.(i) && <span className="text-[11px] text-ink-3">· {when(i)}</span>}
          </div>
          {s.question && (
            <p className="mt-1.5 border-l-2 pl-2.5 text-[13px] font-medium leading-snug text-ink" style={{ borderColor: fogStyle(s.tag).color }}>
              {s.question}
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}

/** Card version for results and Today; renders nothing for clear speech. */
export function FogCard({ signals, title = "Where you seem stuck" }: { signals: FogSignal[]; title?: string }) {
  if (signals.length === 0) return null;
  return (
    <div className="rounded-xl border border-line bg-surface-2/60 p-3 text-left">
      <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-ink-3">{title}</h4>
      <FogList signals={signals} />
    </div>
  );
}
