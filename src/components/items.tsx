// The five kinds of things worth keeping (see engine/sort.rs), with one look
// everywhere: capture results, Today, search and the mind map.

export type ItemKind = "task" | "idea" | "learning" | "question" | "decision";

export const KIND: Record<ItemKind, { label: string; plural: string; color: string; mark: string }> = {
  task: { label: "Task", plural: "Tasks", color: "#16a34a", mark: "✓" },
  idea: { label: "Idea", plural: "Ideas", color: "#d97706", mark: "✦" },
  learning: { label: "Learning", plural: "Learnings", color: "#0d9488", mark: "◆" },
  question: { label: "Question", plural: "Questions", color: "#2563eb", mark: "?" },
  decision: { label: "Decision", plural: "Decisions", color: "#7c3aed", mark: "●" },
};

export const ITEM_KINDS: ItemKind[] = ["idea", "learning", "question", "decision"];

export function kindOf(k: string) {
  return KIND[k as ItemKind] ?? { label: k, plural: k, color: "#64748b", mark: "•" };
}

export function KindBadge({ kind }: { kind: string }) {
  const k = kindOf(kind);
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 py-px text-[10px] font-semibold"
      style={{ background: `${k.color}1a`, color: k.color }}
    >
      <span aria-hidden>{k.mark}</span>
      {k.label}
    </span>
  );
}

/** Items grouped by kind, in a fixed reading order. */
export function ItemGroups({ items }: { items: { id: string; kind: string; text: string }[] }) {
  const order: ItemKind[] = ["decision", "question", "idea", "learning"];
  return (
    <div className="space-y-3">
      {order.map((k) => {
        const list = items.filter((i) => i.kind === k);
        if (list.length === 0) return null;
        return (
          <div key={k}>
            <h4 className="mb-1 text-[11px] font-semibold uppercase tracking-wide" style={{ color: KIND[k].color }}>
              {KIND[k].plural}
            </h4>
            <ul className="space-y-1">
              {list.map((i) => (
                <li key={i.id} className="flex gap-2 text-[13px] leading-snug text-ink">
                  <span className="mt-px shrink-0 text-[11px]" style={{ color: KIND[k].color }} aria-hidden>
                    {KIND[k].mark}
                  </span>
                  {i.text}
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </div>
  );
}
