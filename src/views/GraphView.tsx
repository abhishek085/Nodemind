// The mind map: the ideas, learnings, questions and decisions you've said,
// what they're about, how they connect across days, and where you get stuck.
//
// - Items link to their topics / people / goals ("involves"), to similar
//   items said on other days ("similar", EmbeddingGemma 2, engine/links.rs),
//   and to fog patterns ("fog").
// - Tasks live on Today, raw recordings stay one click away — neither
//   clutters the map.
// - Force-directed layout (d3-force): items cluster around shared topics.
//   Hide all item kinds to get a concept map of topics and people.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import * as api from "../lib/api";
import { LivePartial } from "../components/capture";
import { FogList, fogStyle } from "../components/fog";
import { ITEM_KINDS, KIND, KindBadge, kindOf } from "../components/items";
import { SearchInput, useSearch } from "../components/search";
import type { FogSignal, GraphData, GraphEdge, NodeType, NodeView, Thought } from "../lib/types";

const ENTITY: Record<string, { label: string; color: string }> = {
  topic: { label: "Topics", color: "#7c5ce4" },
  person: { label: "People", color: "#ec4899" },
  goal: { label: "Goals", color: "#f59e0b" },
  fog: { label: "Fog", color: "#dc2626" },
};
const FILTERS: NodeType[] = ["idea", "learning", "question", "decision", "topic", "person", "fog"];
type Range = "7d" | "30d" | "all";

const isItem = (t: string) => (ITEM_KINDS as string[]).includes(t);

function colorOf(n: NodeView): string {
  if (n.nodeType === "fog") return fogStyle(n.label.toLowerCase()).color;
  if (isItem(n.nodeType)) return kindOf(n.nodeType).color;
  return ENTITY[n.nodeType]?.color ?? "#64748b";
}

function filterLabel(t: NodeType): string {
  return isItem(t) ? kindOf(t).plural : (ENTITY[t]?.label ?? t);
}

function filterColor(t: NodeType): string {
  return isItem(t) ? kindOf(t).color : (ENTITY[t]?.color ?? "#64748b");
}

function fmtWhen(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// ── visible subgraph ────────────────────────────────────────────────────────

interface VisEdge {
  id: string;
  source: string;
  target: string;
  kind: "involves" | "similar" | "fog" | "co";
  weight: number;
}

interface Visible {
  nodes: NodeView[];
  edges: VisEdge[];
  /** node id → neighbour ids (hover highlight, details). */
  adj: Map<string, Set<string>>;
}

function visibleGraph(data: GraphData, shown: Set<NodeType>, range: Range): Visible {
  const cutoff = range === "all" ? 0 : Date.now() - (range === "7d" ? 7 : 30) * 86_400_000;
  const byId = new Map(data.nodes.map((n) => [n.id, n]));
  const inRange = (n: NodeView) => !cutoff || new Date(n.lastSeen ?? 0).getTime() >= cutoff;
  // Every in-range item (shown or not) decides which entities are relevant.
  const items = new Set(data.nodes.filter((n) => isItem(n.nodeType) && inRange(n)).map((n) => n.id));
  const shownItems = new Set([...items].filter((id) => shown.has(byId.get(id)!.nodeType)));
  const kindOfEdge = (e: GraphEdge): VisEdge["kind"] =>
    e.edgeType === "similar" ? "similar" : e.edgeType === "fog" ? "fog" : "involves";
  const entityOn = (n: NodeView) => shown.has(n.nodeType) || (n.nodeType === "goal" && shown.has("topic"));

  // Entities (incl. fog hubs) that an in-range item touches → the items.
  const entityItems = new Map<string, Set<string>>();
  for (const e of data.edges) {
    if (kindOfEdge(e) === "similar") continue;
    for (const [a, b] of [[e.fromNode, e.toNode], [e.toNode, e.fromNode]]) {
      const ent = byId.get(b);
      if (items.has(a) && ent && !isItem(ent.nodeType) && entityOn(ent)) {
        if (!entityItems.has(b)) entityItems.set(b, new Set());
        entityItems.get(b)!.add(a);
      }
    }
  }
  // Fog that attached to entities (recordings with no items).
  for (const e of data.edges) {
    if (kindOfEdge(e) !== "fog") continue;
    const [hub, other] = byId.get(e.fromNode)?.nodeType === "fog" ? [e.fromNode, e.toNode] : [e.toNode, e.fromNode];
    if (shown.has("fog") && entityItems.has(other)) {
      if (!entityItems.has(hub)) entityItems.set(hub, new Set());
      for (const i of entityItems.get(other)!) entityItems.get(hub)!.add(i);
    }
  }

  const nodes: NodeView[] = [...[...shownItems].map((id) => byId.get(id)!), ...[...entityItems.keys()].map((id) => byId.get(id)!)];
  const present = new Set(nodes.map((n) => n.id));
  const edges: VisEdge[] = [];
  if (shownItems.size > 0) {
    for (const e of data.edges) {
      if (present.has(e.fromNode) && present.has(e.toNode))
        edges.push({ id: e.id, source: e.fromNode, target: e.toNode, kind: kindOfEdge(e), weight: e.weight });
    }
  } else {
    // Concept map: entities linked when they come up with the same items.
    const ents = [...entityItems.entries()];
    for (let i = 0; i < ents.length; i++)
      for (let j = i + 1; j < ents.length; j++) {
        let shared = 0;
        for (const t of ents[i][1]) if (ents[j][1].has(t)) shared++;
        if (shared > 0) edges.push({ id: `co:${ents[i][0]}:${ents[j][0]}`, source: ents[i][0], target: ents[j][0], kind: "co", weight: shared });
      }
  }
  const adj = new Map<string, Set<string>>();
  for (const e of edges) {
    if (!adj.has(e.source)) adj.set(e.source, new Set());
    if (!adj.has(e.target)) adj.set(e.target, new Set());
    adj.get(e.source)!.add(e.target);
    adj.get(e.target)!.add(e.source);
  }
  return { nodes, edges, adj };
}

// ── force layout ────────────────────────────────────────────────────────────

interface SimNode extends SimulationNodeDatum {
  id: string;
  r: number;
}

function radius(n: NodeView, degree: number): number {
  if (isItem(n.nodeType)) return 80;
  return Math.min(110, 26 + n.label.length * 3.4 + Math.min(degree, 8) * 2);
}

/** Node centres, seeded from the previous layout so filtering doesn't
 *  reshuffle what you were looking at. */
function layout(vis: Visible, prev: Map<string, { x: number; y: number }>): Map<string, { x: number; y: number }> {
  const nodes: SimNode[] = vis.nodes.map((n, i) => {
    const p = prev.get(n.id);
    const a = (i / Math.max(1, vis.nodes.length)) * Math.PI * 2;
    return { id: n.id, r: radius(n, vis.adj.get(n.id)?.size ?? 0), x: p?.x ?? Math.cos(a) * 300, y: p?.y ?? Math.sin(a) * 300 };
  });
  type Link = SimulationLinkDatum<SimNode> & { kind: VisEdge["kind"] };
  const links: Link[] = vis.edges.map((e) => ({ source: e.source, target: e.target, kind: e.kind }));
  forceSimulation(nodes)
    .force(
      "link",
      forceLink<SimNode, Link>(links)
        .id((d) => d.id)
        .distance((l) => (l.kind === "similar" ? 190 : l.kind === "co" ? 190 : 160))
        .strength((l) => (l.kind === "similar" ? 0.5 : 0.35)),
    )
    .force("charge", forceManyBody().strength(-460))
    .force("collide", forceCollide<SimNode>((d) => d.r + 10))
    .force("x", forceX(0).strength(0.05))
    .force("y", forceY(0).strength(0.05))
    .stop()
    .tick(320);
  return new Map(nodes.map((n) => [n.id, { x: n.x ?? 0, y: n.y ?? 0 }]));
}

// ── nodes & edges ───────────────────────────────────────────────────────────

interface MindData extends Record<string, unknown> {
  node: NodeView;
  degree: number;
  dim: boolean;
  focus: boolean;
}
type MindNode = Node<MindData, "entity" | "item">;

// Edges attach at the node centre through invisible handles, so lines run
// centre-to-centre behind the nodes. (Without handles React Flow draws no
// edges at all — why the first map never showed its links.)
const CENTER = {
  left: "50%",
  top: "50%",
  transform: "translate(-50%, -50%)",
  width: 1,
  height: 1,
  minWidth: 0,
  minHeight: 0,
  border: 0,
  opacity: 0,
} as const;

function Handles() {
  return (
    <>
      <Handle type="source" position={Position.Top} style={CENTER} isConnectable={false} />
      <Handle type="target" position={Position.Top} style={CENTER} isConnectable={false} />
    </>
  );
}

function EntityNode({ data }: NodeProps<MindNode>) {
  const { node, degree, dim, focus } = data;
  const color = colorOf(node);
  return (
    <div
      className="flex items-center gap-1.5 rounded-full border px-3 py-1.5 transition-opacity duration-200"
      style={{
        opacity: dim ? 0.15 : 1,
        background: `color-mix(in srgb, ${color} 14%, var(--surface))`,
        borderColor: focus ? color : `${color}66`,
        boxShadow: focus ? `0 0 0 3px ${color}33` : "0 1px 2px rgba(0,0,0,.06)",
        fontSize: Math.min(16, 12 + degree * 0.6),
      }}
    >
      <Handles />
      {node.nodeType === "person" && <span className="text-[0.85em]">👤</span>}
      <span className="whitespace-nowrap font-semibold" style={{ color }}>
        {node.nodeType === "fog" ? fogStyle(node.label.toLowerCase()).label : node.label}
      </span>
      {degree > 1 && (
        <span className="rounded-full px-1.5 text-[10px] font-bold text-white" style={{ background: color }}>
          {degree}
        </span>
      )}
    </div>
  );
}

function ItemNode({ data }: NodeProps<MindNode>) {
  const { node, dim, focus } = data;
  const color = colorOf(node);
  return (
    <div
      className="w-48 rounded-xl border bg-surface px-3 py-2 shadow-sm transition-opacity duration-200"
      style={{
        opacity: dim ? 0.15 : 1,
        borderColor: focus ? color : "var(--line)",
        borderLeft: `3px solid ${color}`,
        boxShadow: focus ? `0 0 0 3px ${color}33` : undefined,
      }}
    >
      <Handles />
      <div className="mb-0.5 flex items-center justify-between gap-2">
        <KindBadge kind={node.nodeType} />
        <span className="text-[10px] text-ink-3">{fmtWhen(node.lastSeen)}</span>
      </div>
      <p className="line-clamp-3 text-[12px] leading-snug text-ink">{node.label}</p>
    </div>
  );
}

const nodeTypes: NodeTypes = { entity: EntityNode, item: ItemNode };

function edgeStyle(e: VisEdge, dim: boolean, fogColor: string): Edge["style"] {
  const base = { opacity: dim ? 0.06 : 0.75, transition: "opacity 200ms" };
  switch (e.kind) {
    case "similar":
      return { ...base, stroke: "#4f6ef5", strokeWidth: 1 + Math.min(3, e.weight * 0.4), strokeDasharray: "6 4" };
    case "fog":
      return { ...base, stroke: fogColor, strokeWidth: 1.5, strokeDasharray: "2 4" };
    case "co":
      return { ...base, stroke: "var(--line-strong)", strokeWidth: Math.min(5, 1 + e.weight) };
    default:
      return { ...base, stroke: "var(--line-strong)", strokeWidth: 1.2 };
  }
}

// ── view ────────────────────────────────────────────────────────────────────

interface Props {
  refreshKey: number;
  /** Item to spotlight on open (from Search → "Show in mind map"). */
  focusId?: string | null;
}

export default function GraphView(props: Props) {
  return (
    <ReactFlowProvider>
      <MindMap {...props} />
    </ReactFlowProvider>
  );
}

function MindMap({ refreshKey, focusId = null }: Props) {
  const [data, setData] = useState<GraphData | null>(null);
  const [shown, setShown] = useState<Set<NodeType>>(new Set(FILTERS));
  const [range, setRange] = useState<Range>("all");
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const positions = useRef(new Map<string, { x: number; y: number }>());
  const flow = useReactFlow();

  useEffect(() => {
    void api
      .getGraph()
      .then(setData)
      .catch(() => setData({ nodes: [], edges: [] }));
  }, [refreshKey]);

  const vis = useMemo(() => (data ? visibleGraph(data, shown, range) : null), [data, shown, range]);
  const pos = useMemo(() => {
    if (!vis) return new Map<string, { x: number; y: number }>();
    const p = layout(vis, positions.current);
    positions.current = new Map([...positions.current, ...p]);
    return p;
  }, [vis]);

  const focusNode = useCallback(
    (id: string) => {
      setSelected(id);
      const p = pos.get(id);
      if (p) void flow.setCenter(p.x, p.y, { zoom: Math.max(flow.getZoom(), 0.9), duration: 450 });
    },
    [pos, flow],
  );

  useEffect(() => {
    if (focusId && pos.has(focusId)) focusNode(focusId);
  }, [focusId, pos, focusNode]);

  const active = hover ?? selected;
  const near = useMemo(() => {
    if (!active || !vis) return null;
    return new Set([active, ...(vis.adj.get(active) ?? [])]);
  }, [active, vis]);

  const byId = useMemo(() => new Map((data?.nodes ?? []).map((n) => [n.id, n])), [data]);

  const nodes: MindNode[] = useMemo(
    () =>
      (vis?.nodes ?? []).map((n) => ({
        id: n.id,
        type: isItem(n.nodeType) ? "item" : "entity",
        position: pos.get(n.id) ?? { x: 0, y: 0 },
        data: { node: n, degree: vis?.adj.get(n.id)?.size ?? 0, dim: !!near && !near.has(n.id), focus: n.id === selected },
      })),
    [vis, pos, near, selected],
  );

  const edges: Edge[] = useMemo(
    () =>
      (vis?.edges ?? []).map((e) => {
        const dim = !!active && e.source !== active && e.target !== active;
        const hub = e.kind === "fog" ? [byId.get(e.source), byId.get(e.target)].find((n) => n?.nodeType === "fog") : undefined;
        return { id: e.id, source: e.source, target: e.target, type: "straight", style: edgeStyle(e, dim, hub ? colorOf(hub) : "#dc2626"), zIndex: 0 };
      }),
    [vis, active, byId],
  );

  const toggle = (t: NodeType) =>
    setShown((s) => {
      const next = new Set(s);
      if (next.has(t)) next.delete(t);
      else next.add(t);
      return next;
    });

  return (
    <div className="relative flex flex-1 overflow-hidden">
      <div className="relative flex-1">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          nodeOrigin={[0.5, 0.5]}
          onNodeMouseEnter={(_, n) => setHover(n.id)}
          onNodeMouseLeave={() => setHover(null)}
          onNodeClick={(_, n) => setSelected(n.id)}
          onPaneClick={() => setSelected(null)}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          minZoom={0.2}
          maxZoom={2}
          nodesConnectable={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--line-strong)" />
          <Controls showInteractive={false} />
        </ReactFlow>

        <div className="no-drag absolute left-3 top-3 flex max-w-[calc(100%-1.5rem)] flex-wrap items-center gap-1.5">
          {FILTERS.map((t) => (
            <button
              key={t}
              onClick={() => toggle(t)}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-colors ${
                shown.has(t) ? "border-line bg-surface text-ink shadow-sm" : "border-transparent bg-surface-2 text-ink-3"
              }`}
            >
              <span className="h-2 w-2 rounded-full" style={{ background: shown.has(t) ? filterColor(t) : "var(--line-strong)" }} />
              {filterLabel(t)}
            </button>
          ))}
          <span className="mx-1 h-4 w-px bg-line" />
          {(["7d", "30d", "all"] as Range[]).map((r) => (
            <button
              key={r}
              onClick={() => setRange(r)}
              className={`rounded-full px-2.5 py-1 text-[11px] font-medium ${range === r ? "bg-accent text-white" : "bg-surface-2 text-ink-3 hover:text-ink-2"}`}
            >
              {r === "all" ? "All time" : r === "7d" ? "7 days" : "30 days"}
            </button>
          ))}
        </div>

        <div className="pointer-events-none absolute bottom-3 right-3 flex gap-3 rounded-lg bg-surface/90 px-3 py-1.5 text-[10px] text-ink-3 shadow-sm">
          <Legend stroke="var(--line-strong)" label="about" />
          <Legend stroke="#4f6ef5" dash="5 3" label="similar" />
          <Legend stroke="#dc2626" dash="2 3" label="fog" />
        </div>

        {vis && vis.nodes.length === 0 && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <p className="max-w-xs text-center text-sm text-ink-3">
              {data && data.nodes.length > 0
                ? "Nothing matches these filters."
                : "Your ideas, questions, decisions and learnings show up here as you talk. Hold ⌥⇧Space and think out loud."}
            </p>
          </div>
        )}
      </div>

      <SidePanel data={data} vis={vis} selected={selected} onSelect={focusNode} />
    </div>
  );
}

function Legend({ stroke, dash, label }: { stroke: string; dash?: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <svg width="18" height="4">
        <line x1="0" y1="2" x2="18" y2="2" stroke={stroke} strokeWidth="1.5" strokeDasharray={dash} />
      </svg>
      {label}
    </span>
  );
}

// ── side panel ──────────────────────────────────────────────────────────────

function SidePanel({
  data,
  vis,
  selected,
  onSelect,
}: {
  data: GraphData | null;
  vis: Visible | null;
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const search = useSearch(10);
  const byId = useMemo(() => new Map((data?.nodes ?? []).map((n) => [n.id, n])), [data]);
  const node = selected ? byId.get(selected) : undefined;

  return (
    <aside className="no-drag flex w-80 shrink-0 flex-col border-l border-line bg-surface-2">
      <div className="border-b border-line p-3">
        <SearchInput value={search.query} onChange={search.setQuery} busy={search.busy} placeholder="Search by meaning…" />
        {search.query.trim().length >= 2 && (
          <ul className="mt-2 max-h-64 space-y-0.5 overflow-y-auto">
            {search.hits.map((h) => (
              <li key={h.id}>
                <button
                  onClick={() => byId.has(h.id) && onSelect(h.id)}
                  className={`block w-full rounded-lg px-2 py-1.5 text-left ${byId.has(h.id) ? "hover:bg-surface" : "cursor-default"}`}
                  title={byId.has(h.id) ? "Show on the map" : "Tasks live on Today"}
                >
                  <div className="flex items-center gap-1.5">
                    <KindBadge kind={h.kind} />
                    <span className="text-[10px] text-ink-3">{fmtWhen(h.saidAt)}</span>
                  </div>
                  <p className="mt-0.5 line-clamp-2 text-xs text-ink">{h.text}</p>
                </button>
              </li>
            ))}
            {search.error && <li className="px-2 py-1 text-xs text-danger">{search.error}</li>}
            {search.hits.length === 0 && !search.busy && !search.error && (
              <li className="px-2 py-1 text-xs text-ink-3">Nothing about that yet.</li>
            )}
          </ul>
        )}
      </div>
      <div className="flex-1 overflow-y-auto p-4">
        {!node ? (
          <Overview vis={vis} onSelect={onSelect} />
        ) : isItem(node.nodeType) ? (
          <ItemDetail node={node} vis={vis} byId={byId} onSelect={onSelect} />
        ) : (
          <EntityDetail node={node} vis={vis} byId={byId} onSelect={onSelect} />
        )}
      </div>
    </aside>
  );
}

function SectionTitle({ children, color }: { children: string; color?: string }) {
  return (
    <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-wide text-ink-3" style={color ? { color } : undefined}>
      {children}
    </p>
  );
}

function Chip({ n, onSelect, count }: { n: NodeView; onSelect: (id: string) => void; count?: number }) {
  const color = colorOf(n);
  return (
    <button
      onClick={() => onSelect(n.id)}
      className="rounded-full px-2 py-0.5 text-[11px] font-medium"
      style={{ background: `color-mix(in srgb, ${color} 14%, transparent)`, color }}
    >
      {n.nodeType === "fog" ? fogStyle(n.label.toLowerCase()).label : n.label}
      {count !== undefined && <span className="ml-1 opacity-60">{count}</span>}
    </button>
  );
}

function ItemRow({ n, onSelect }: { n: NodeView; onSelect: (id: string) => void }) {
  return (
    <button onClick={() => onSelect(n.id)} className="flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface">
      <span className="mt-0.5 shrink-0 text-[11px]" style={{ color: colorOf(n) }} aria-hidden>
        {kindOf(n.nodeType).mark}
      </span>
      <span className="min-w-0 flex-1">
        <span className="line-clamp-2 text-xs text-ink">{n.label}</span>
        <span className="text-[10px] text-ink-3">{fmtWhen(n.lastSeen)}</span>
      </span>
    </button>
  );
}

/** Nothing selected: what's worth your attention, at a glance. */
function Overview({ vis, onSelect }: { vis: Visible | null; onSelect: (id: string) => void }) {
  if (!vis) return null;
  const degree = (n: NodeView) => vis.adj.get(n.id)?.size ?? 0;
  const newest = (kind: string, max: number) =>
    vis.nodes
      .filter((n) => n.nodeType === kind)
      .sort((a, b) => (b.lastSeen ?? "").localeCompare(a.lastSeen ?? ""))
      .slice(0, max);
  const top = (type: NodeType) =>
    vis.nodes
      .filter((n) => n.nodeType === type)
      .sort((a, b) => degree(b) - degree(a))
      .slice(0, 8);
  const questions = newest("question", 5);
  const ideas = newest("idea", 5);
  const topics = top("topic");
  const fog = top("fog");
  const empty = !questions.length && !ideas.length && !topics.length && !fog.length;
  return (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-bold">Your mind map</h3>
        <p className="mt-1 text-xs leading-relaxed text-ink-3">
          Hover anything to see what it connects to; click for details.
        </p>
      </div>
      {questions.length > 0 && (
        <div>
          <SectionTitle color={KIND.question.color}>Open questions</SectionTitle>
          {questions.map((n) => (
            <ItemRow key={n.id} n={n} onSelect={onSelect} />
          ))}
        </div>
      )}
      {ideas.length > 0 && (
        <div>
          <SectionTitle color={KIND.idea.color}>Recent ideas</SectionTitle>
          {ideas.map((n) => (
            <ItemRow key={n.id} n={n} onSelect={onSelect} />
          ))}
        </div>
      )}
      {topics.length > 0 && (
        <div>
          <SectionTitle>On your mind most</SectionTitle>
          <div className="flex flex-wrap gap-1.5">
            {topics.map((n) => (
              <Chip key={n.id} n={n} onSelect={onSelect} count={degree(n)} />
            ))}
          </div>
        </div>
      )}
      {fog.length > 0 && (
        <div>
          <SectionTitle>Recurring fog</SectionTitle>
          <div className="flex flex-wrap gap-1.5">
            {fog.map((n) => (
              <Chip key={n.id} n={n} onSelect={onSelect} count={degree(n)} />
            ))}
          </div>
        </div>
      )}
      {empty && <p className="text-xs text-ink-3">Nothing here yet.</p>}
    </div>
  );
}

function parseFog(t: Thought | null): FogSignal[] {
  try {
    return t?.fog ? (JSON.parse(t.fog) as FogSignal[]) : [];
  } catch {
    return [];
  }
}

interface DetailProps {
  node: NodeView;
  vis: Visible | null;
  byId: Map<string, NodeView>;
  onSelect: (id: string) => void;
}

function ItemDetail({ node, vis, byId, onSelect }: DetailProps) {
  const [source, setSource] = useState<Thought | null>(null);
  useEffect(() => {
    setSource(null);
    if (node.thoughtId)
      void api
        .getThought(node.thoughtId)
        .then(setSource)
        .catch(() => undefined);
  }, [node.thoughtId]);

  const neighbours = [...(vis?.adj.get(node.id) ?? [])].map((id) => byId.get(id)).filter((n): n is NodeView => !!n);
  const about = neighbours.filter((n) => !isItem(n.nodeType) && n.nodeType !== "fog");
  const similar = neighbours.filter((n) => isItem(n.nodeType));
  const fog = parseFog(source);

  return (
    <div className="space-y-4">
      <div>
        <div className="flex items-center gap-2">
          <KindBadge kind={node.nodeType} />
          <span className="text-[11px] text-ink-3">{fmtWhen(node.lastSeen)}</span>
        </div>
        <p className="mt-1.5 text-sm font-semibold leading-snug text-ink">{node.label}</p>
      </div>
      {about.length > 0 && (
        <div>
          <SectionTitle>About</SectionTitle>
          <div className="flex flex-wrap gap-1.5">
            {about.map((n) => (
              <Chip key={n.id} n={n} onSelect={onSelect} />
            ))}
          </div>
        </div>
      )}
      {similar.length > 0 && (
        <div>
          <SectionTitle>Connects to</SectionTitle>
          {similar.map((n) => (
            <ItemRow key={n.id} n={n} onSelect={onSelect} />
          ))}
        </div>
      )}
      {fog.length > 0 && (
        <div>
          <SectionTitle>Where you were stuck</SectionTitle>
          <FogList signals={fog} />
        </div>
      )}
      {source && (
        <details className="text-xs text-ink-3">
          <summary className="cursor-pointer">What you said</summary>
          <LivePartial text={source.text} className="mt-1 max-h-64" />
        </details>
      )}
    </div>
  );
}

function EntityDetail({ node, vis, byId, onSelect }: DetailProps) {
  const items = useMemo(
    () =>
      [...(vis?.adj.get(node.id) ?? [])]
        .map((id) => byId.get(id))
        .filter((n): n is NodeView => !!n && isItem(n.nodeType))
        .sort((a, b) => (b.lastSeen ?? "").localeCompare(a.lastSeen ?? "")),
    [vis, node.id, byId],
  );
  const related = useMemo(() => {
    const together = new Map<string, number>();
    for (const t of items)
      for (const id of vis?.adj.get(t.id) ?? [])
        if (id !== node.id && !isItem(byId.get(id)?.nodeType ?? "")) together.set(id, (together.get(id) ?? 0) + 1);
    return [...together.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8)
      .map(([id, c]) => ({ node: byId.get(id), c }))
      .filter((x): x is { node: NodeView; c: number } => !!x.node);
  }, [items, vis, node.id, byId]);

  const isFog = node.nodeType === "fog";
  const fogKind = fogStyle(node.label.toLowerCase());
  const color = colorOf(node);

  // For a fog pattern: what each recording was stuck on, and its question.
  const [fogBySource, setFogBySource] = useState<Map<string, FogSignal[]>>(new Map());
  useEffect(() => {
    if (!isFog) return;
    let live = true;
    const sources = [...new Set(items.map((i) => i.thoughtId).filter((t): t is string => !!t))].slice(0, 12);
    void Promise.all(sources.map((t) => api.getThought(t).catch(() => null))).then((ts) => {
      if (!live) return;
      const m = new Map<string, FogSignal[]>();
      for (const t of ts) if (t) m.set(t.id, parseFog(t).filter((f) => fogStyle(f.tag).label === fogKind.label));
      setFogBySource(m);
    });
    return () => {
      live = false;
    };
  }, [isFog, items, fogKind.label]);

  const n = items.length;
  return (
    <div className="space-y-4">
      <div>
        <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color }}>
          {isFog ? "Fog pattern" : (ENTITY[node.nodeType]?.label.replace(/s$/, "") ?? node.nodeType)}
        </p>
        <h3 className="mt-0.5 text-base font-bold">{isFog ? fogKind.label : node.label}</h3>
        <p className="text-xs text-ink-3">
          {isFog
            ? `Around ${n} thing${n === 1 ? "" : "s"} you said${fogKind.hint ? ` — ${fogKind.hint}` : ""}`
            : `${n} thing${n === 1 ? "" : "s"} you said about it`}
        </p>
      </div>
      {isFog && [...fogBySource.values()].some((l) => l.length > 0) && (
        <div>
          <SectionTitle>Questions to get unstuck</SectionTitle>
          <FogList signals={[...fogBySource.values()].flat()} />
        </div>
      )}
      {related.length > 0 && (
        <div>
          <SectionTitle>Comes up with</SectionTitle>
          <div className="flex flex-wrap gap-1.5">
            {related.map(({ node: r, c }) => (
              <Chip key={r.id} n={r} onSelect={onSelect} count={c} />
            ))}
          </div>
        </div>
      )}
      {(["decision", "question", "idea", "learning"] as const).map((k) => {
        const list = items.filter((i) => i.nodeType === k);
        if (list.length === 0) return null;
        return (
          <div key={k}>
            <SectionTitle color={KIND[k].color}>{KIND[k].plural}</SectionTitle>
            {list.map((i) => (
              <ItemRow key={i.id} n={i} onSelect={onSelect} />
            ))}
          </div>
        );
      })}
    </div>
  );
}
