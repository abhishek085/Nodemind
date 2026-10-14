// The mind map: what you keep thinking about, how your thoughts connect,
// and where you get stuck.
//
// - Thoughts link to their topics / people / goals ("involves") and to fog
//   patterns ("fog"); similar thoughts link to each other ("similar",
//   decided by EmbeddingGemma 2 in engine/links.rs).
// - Layout is force-directed (d3-force), so thoughts cluster around what
//   they share. Hide thoughts to get a concept map: entities linked when they
//   come up together.

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
import { SearchInput, useSearch } from "../components/search";
import type { FogSignal, GraphData, GraphEdge, NodeType, NodeView, Thought } from "../lib/types";

const TYPE: Record<string, { label: string; color: string }> = {
  thought: { label: "Thoughts", color: "#4f6ef5" },
  topic: { label: "Topics", color: "#7c5ce4" },
  person: { label: "People", color: "#ec4899" },
  goal: { label: "Goals", color: "#f59e0b" },
  fog: { label: "Fog", color: "#dc2626" },
};
const FILTERS: NodeType[] = ["thought", "topic", "person", "goal", "fog"];
type Range = "7d" | "30d" | "all";

function colorOf(n: NodeView): string {
  return n.nodeType === "fog" ? fogStyle(n.label.toLowerCase()).color : (TYPE[n.nodeType]?.color ?? "#64748b");
}

function titleOf(n: NodeView): string {
  return n.nodeType === "thought" ? n.summary || n.label : n.label;
}

function fmtWhen(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return (
    d.toLocaleDateString(undefined, { month: "short", day: "numeric" }) +
    " · " +
    d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  );
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
  const thoughts = new Set(
    data.nodes
      .filter((n) => n.nodeType === "thought" && (!cutoff || new Date(n.lastSeen ?? 0).getTime() >= cutoff))
      .map((n) => n.id),
  );
  const kindOf = (e: GraphEdge): VisEdge["kind"] =>
    e.edgeType === "similar" ? "similar" : e.edgeType === "fog" ? "fog" : "involves";

  // Entities show when their type is on and an in-range thought mentions them.
  const entityThoughts = new Map<string, Set<string>>();
  for (const e of data.edges) {
    if (kindOf(e) === "similar") continue;
    const [t, x] = thoughts.has(e.fromNode) ? [e.fromNode, e.toNode] : [e.toNode, e.fromNode];
    const ent = byId.get(x);
    if (!thoughts.has(t) || !ent || ent.nodeType === "thought" || !shown.has(ent.nodeType)) continue;
    if (!entityThoughts.has(x)) entityThoughts.set(x, new Set());
    entityThoughts.get(x)!.add(t);
  }

  const showThoughts = shown.has("thought");
  const nodes: NodeView[] = [
    ...(showThoughts ? [...thoughts].map((id) => byId.get(id)!) : []),
    ...[...entityThoughts.keys()].map((id) => byId.get(id)!),
  ];
  const edges: VisEdge[] = [];
  if (showThoughts) {
    for (const e of data.edges) {
      const kind = kindOf(e);
      const ok =
        kind === "similar"
          ? thoughts.has(e.fromNode) && thoughts.has(e.toNode)
          : (thoughts.has(e.fromNode) && entityThoughts.has(e.toNode)) ||
            (thoughts.has(e.toNode) && entityThoughts.has(e.fromNode));
      if (ok) edges.push({ id: e.id, source: e.fromNode, target: e.toNode, kind, weight: e.weight });
    }
  } else {
    // Concept map: entities that come up in the same thoughts are linked.
    const ents = [...entityThoughts.entries()];
    for (let i = 0; i < ents.length; i++) {
      for (let j = i + 1; j < ents.length; j++) {
        let shared = 0;
        for (const t of ents[i][1]) if (ents[j][1].has(t)) shared++;
        if (shared > 0)
          edges.push({ id: `co:${ents[i][0]}:${ents[j][0]}`, source: ents[i][0], target: ents[j][0], kind: "co", weight: shared });
      }
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
  if (n.nodeType === "thought") return 70;
  return Math.min(110, 26 + titleOf(n).length * 3.4 + Math.min(degree, 8) * 2);
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
  const sim = forceSimulation(nodes)
    .force(
      "link",
      forceLink<SimNode, Link>(links)
        .id((d) => d.id)
        .distance((l) => (l.kind === "similar" ? 170 : l.kind === "co" ? 190 : 150))
        .strength((l) => (l.kind === "similar" ? 0.5 : 0.35)),
    )
    .force("charge", forceManyBody().strength(-420))
    .force("collide", forceCollide<SimNode>((d) => d.r + 10))
    .force("x", forceX(0).strength(0.05))
    .force("y", forceY(0).strength(0.05))
    .stop();
  sim.tick(320);
  return new Map(nodes.map((n) => [n.id, { x: n.x ?? 0, y: n.y ?? 0 }]));
}

// ── nodes & edges ───────────────────────────────────────────────────────────

interface MindData extends Record<string, unknown> {
  node: NodeView;
  degree: number;
  dim: boolean;
  focus: boolean;
}
type MindNode = Node<MindData, "entity" | "thought">;

// Edges attach at the node centre through invisible handles, so lines run
// centre-to-centre behind the nodes whatever the layout. (Without handles,
// React Flow can't draw an edge at all — the old map's links never showed.)
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
        {node.label}
      </span>
      {degree > 1 && (
        <span className="rounded-full px-1.5 text-[10px] font-bold text-white" style={{ background: color }}>
          {degree}
        </span>
      )}
    </div>
  );
}

function ThoughtNode({ data }: NodeProps<MindNode>) {
  const { node, dim, focus } = data;
  return (
    <div
      className="w-44 rounded-xl border bg-surface px-3 py-2 shadow-sm transition-opacity duration-200"
      style={{
        opacity: dim ? 0.15 : 1,
        borderColor: focus ? TYPE.thought.color : "var(--line)",
        borderLeft: `3px solid ${TYPE.thought.color}`,
        boxShadow: focus ? `0 0 0 3px ${TYPE.thought.color}33` : undefined,
      }}
    >
      <Handles />
      <p className="text-[10px] text-ink-3">{fmtWhen(node.lastSeen)}</p>
      <p className="line-clamp-2 text-[12px] font-medium leading-snug text-ink">{titleOf(node)}</p>
    </div>
  );
}

const nodeTypes: NodeTypes = { entity: EntityNode, thought: ThoughtNode };

function edgeStyle(e: VisEdge, dim: boolean, fogColor: string): Edge["style"] {
  const base = { opacity: dim ? 0.06 : 0.75, transition: "opacity 200ms" };
  switch (e.kind) {
    case "similar":
      return { ...base, stroke: TYPE.thought.color, strokeWidth: 1 + Math.min(3, e.weight * 0.4), strokeDasharray: "6 4" };
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
  /** Thought to spotlight on open (from Search → "Show in mind map"). */
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
        type: n.nodeType === "thought" ? "thought" : "entity",
        position: pos.get(n.id) ?? { x: 0, y: 0 },
        data: { node: n, degree: vis?.adj.get(n.id)?.size ?? 0, dim: !!near && !near.has(n.id), focus: n.id === selected },
      })),
    [vis, pos, near, selected],
  );

  const edges: Edge[] = useMemo(
    () =>
      (vis?.edges ?? []).map((e) => {
        const dim = !!active && e.source !== active && e.target !== active;
        const fogNode = e.kind === "fog" ? [byId.get(e.source), byId.get(e.target)].find((n) => n?.nodeType === "fog") : undefined;
        return {
          id: e.id,
          source: e.source,
          target: e.target,
          type: "straight",
          style: edgeStyle(e, dim, fogNode ? colorOf(fogNode) : "#dc2626"),
          zIndex: 0,
        };
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

        <div className="no-drag absolute left-3 top-3 flex flex-wrap items-center gap-1.5">
          {FILTERS.map((t) => (
            <button
              key={t}
              onClick={() => toggle(t)}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-colors ${
                shown.has(t) ? "border-line bg-surface text-ink shadow-sm" : "border-transparent bg-surface-2 text-ink-3"
              }`}
            >
              <span className="h-2 w-2 rounded-full" style={{ background: shown.has(t) ? TYPE[t].color : "var(--line-strong)" }} />
              {TYPE[t].label}
            </button>
          ))}
          <span className="mx-1 h-4 w-px bg-line" />
          {(["7d", "30d", "all"] as Range[]).map((r) => (
            <button
              key={r}
              onClick={() => setRange(r)}
              className={`rounded-full px-2.5 py-1 text-[11px] font-medium ${
                range === r ? "bg-accent text-white" : "bg-surface-2 text-ink-3 hover:text-ink-2"
              }`}
            >
              {r === "all" ? "All time" : r === "7d" ? "7 days" : "30 days"}
            </button>
          ))}
        </div>

        <div className="pointer-events-none absolute bottom-3 right-3 flex gap-3 rounded-lg bg-surface/90 px-3 py-1.5 text-[10px] text-ink-3 shadow-sm">
          <Legend stroke="var(--line-strong)" label="mentions" />
          <Legend stroke={TYPE.thought.color} dash="5 3" label="similar thought" />
          <Legend stroke="#dc2626" dash="2 3" label="fog" />
        </div>

        {vis && vis.nodes.length === 0 && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <p className="max-w-xs text-center text-sm text-ink-3">
              {data && data.nodes.length > 0
                ? "Nothing matches these filters."
                : "Your mind map fills in as you talk. Hold ⌥⇧Space and brain-dump."}
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
  const search = useSearch(8);
  const byId = useMemo(() => new Map((data?.nodes ?? []).map((n) => [n.id, n])), [data]);
  const node = selected ? byId.get(selected) : undefined;

  return (
    <aside className="no-drag flex w-80 shrink-0 flex-col border-l border-line bg-surface-2">
      <div className="border-b border-line p-3">
        <SearchInput value={search.query} onChange={search.setQuery} busy={search.busy} placeholder="Find a thought by meaning…" />
        {search.query.trim().length >= 2 && (
          <ul className="mt-2 max-h-56 space-y-1 overflow-y-auto">
            {search.hits.map((h) => (
              <li key={h.thoughtId}>
                <button onClick={() => onSelect(h.thoughtId)} className="block w-full rounded-lg px-2 py-1.5 text-left hover:bg-surface">
                  <p className="text-[10px] text-ink-3">
                    {fmtWhen(h.startedAt)}
                    {h.strong && " · strong match"}
                  </p>
                  <p className="line-clamp-2 text-xs text-ink">{h.summary ?? h.snippet}</p>
                </button>
              </li>
            ))}
            {search.hits.length === 0 && !search.busy && <li className="px-2 py-1 text-xs text-ink-3">No match yet.</li>}
          </ul>
        )}
      </div>
      <div className="flex-1 overflow-y-auto p-4">
        {!node ? (
          <Overview vis={vis} onSelect={onSelect} />
        ) : node.nodeType === "thought" ? (
          <ThoughtDetail node={node} vis={vis} byId={byId} onSelect={onSelect} />
        ) : (
          <EntityDetail node={node} vis={vis} byId={byId} onSelect={onSelect} />
        )}
      </div>
    </aside>
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
      {n.label}
      {count !== undefined && <span className="ml-1 opacity-60">{count}</span>}
    </button>
  );
}

function ThoughtLink({ n, onSelect }: { n: NodeView; onSelect: (id: string) => void }) {
  return (
    <button onClick={() => onSelect(n.id)} className="block w-full rounded-lg px-2 py-1.5 text-left hover:bg-surface">
      <p className="text-[10px] text-ink-3">{fmtWhen(n.lastSeen)}</p>
      <p className="line-clamp-2 text-xs text-ink">{titleOf(n)}</p>
    </button>
  );
}

function SectionTitle({ children }: { children: string }) {
  return <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-wide text-ink-3">{children}</p>;
}

/** Nothing selected: the map at a glance. */
function Overview({ vis, onSelect }: { vis: Visible | null; onSelect: (id: string) => void }) {
  if (!vis) return null;
  const degree = (n: NodeView) => vis.adj.get(n.id)?.size ?? 0;
  const ranked = (type: NodeType) =>
    vis.nodes
      .filter((n) => n.nodeType === type)
      .sort((a, b) => degree(b) - degree(a))
      .slice(0, 8);
  const thoughts = vis.nodes.filter((n) => n.nodeType === "thought").length;
  const similar = vis.edges.filter((e) => e.kind === "similar").length;
  const sections: [string, NodeType][] = [
    ["On your mind most", "topic"],
    ["People", "person"],
    ["Goals", "goal"],
    ["Recurring fog", "fog"],
  ];
  return (
    <div className="space-y-5">
      <div>
        <h3 className="text-sm font-bold">Your mind map</h3>
        <p className="mt-1 text-xs leading-relaxed text-ink-3">
          {thoughts} thoughts · {similar} links between similar thoughts. Hover anything to see what it touches; click for details.
        </p>
      </div>
      {sections.map(([title, type]) => {
        const list = ranked(type);
        if (list.length === 0) return null;
        return (
          <div key={type}>
            <SectionTitle>{title}</SectionTitle>
            <div className="flex flex-wrap gap-1.5">
              {list.map((n) => (
                <Chip key={n.id} n={n} onSelect={onSelect} count={degree(n)} />
              ))}
            </div>
          </div>
        );
      })}
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

function ThoughtDetail({ node, vis, byId, onSelect }: DetailProps) {
  const [thought, setThought] = useState<Thought | null>(null);
  useEffect(() => {
    setThought(null);
    void api
      .getThought(node.id)
      .then(setThought)
      .catch(() => undefined);
  }, [node.id]);

  const neighbours = [...(vis?.adj.get(node.id) ?? [])].map((id) => byId.get(id)).filter((n): n is NodeView => !!n);
  const entities = neighbours.filter((n) => n.nodeType !== "thought" && n.nodeType !== "fog");
  const similar = neighbours.filter((n) => n.nodeType === "thought");
  const fog = parseFog(thought);

  return (
    <div className="space-y-4">
      <div>
        <p className="text-[11px] text-ink-3">
          {fmtWhen(node.lastSeen)} · {thought?.kind === "meeting" ? "Meeting" : "Note"}
        </p>
        <h3 className="mt-0.5 text-sm font-bold leading-snug">{titleOf(node)}</h3>
      </div>
      {entities.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {entities.map((n) => (
            <Chip key={n.id} n={n} onSelect={onSelect} />
          ))}
        </div>
      )}
      {fog.length > 0 && (
        <div>
          <SectionTitle>Where you were stuck</SectionTitle>
          <FogList signals={fog} />
        </div>
      )}
      {similar.length > 0 && (
        <div>
          <SectionTitle>Similar thoughts</SectionTitle>
          {similar.map((n) => (
            <ThoughtLink key={n.id} n={n} onSelect={onSelect} />
          ))}
        </div>
      )}
      {thought && (
        <details className="text-xs text-ink-3">
          <summary className="cursor-pointer">Transcript</summary>
          <LivePartial text={thought.text} className="mt-1 max-h-64" />
        </details>
      )}
    </div>
  );
}

function EntityDetail({ node, vis, byId, onSelect }: DetailProps) {
  const thoughts = useMemo(
    () =>
      [...(vis?.adj.get(node.id) ?? [])]
        .map((id) => byId.get(id))
        .filter((n): n is NodeView => !!n && n.nodeType === "thought")
        .sort((a, b) => (b.lastSeen ?? "").localeCompare(a.lastSeen ?? "")),
    [vis, node.id, byId],
  );
  // What else comes up alongside it.
  const related = useMemo(() => {
    const together = new Map<string, number>();
    for (const t of thoughts)
      for (const id of vis?.adj.get(t.id) ?? [])
        if (id !== node.id && byId.get(id)?.nodeType !== "thought") together.set(id, (together.get(id) ?? 0) + 1);
    return [...together.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8)
      .map(([id, n]) => ({ node: byId.get(id), n }))
      .filter((x): x is { node: NodeView; n: number } => !!x.node);
  }, [thoughts, vis, node.id, byId]);

  const isFog = node.nodeType === "fog";
  const fogKind = fogStyle(node.label.toLowerCase());
  const color = colorOf(node);

  // For a fog pattern: what each thought was stuck on, and its question.
  const [fogByThought, setFogByThought] = useState<Map<string, FogSignal[]>>(new Map());
  useEffect(() => {
    if (!isFog) return;
    let live = true;
    void Promise.all(thoughts.slice(0, 12).map((t) => api.getThought(t.id).catch(() => null))).then((ts) => {
      if (!live) return;
      const m = new Map<string, FogSignal[]>();
      for (const t of ts) if (t) m.set(t.id, parseFog(t).filter((f) => fogStyle(f.tag).label === fogKind.label));
      setFogByThought(m);
    });
    return () => {
      live = false;
    };
  }, [isFog, thoughts, fogKind.label]);

  const count = `${thoughts.length} thought${thoughts.length === 1 ? "" : "s"}`;
  return (
    <div className="space-y-4">
      <div>
        <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color }}>
          {isFog ? "Fog pattern" : (TYPE[node.nodeType]?.label.replace(/s$/, "") ?? node.nodeType)}
        </p>
        <h3 className="mt-0.5 text-base font-bold">{node.label}</h3>
        <p className="text-xs text-ink-3">
          {isFog ? `Came up in ${count}${fogKind.hint ? ` — ${fogKind.hint}` : ""}` : `Mentioned in ${count}`}
        </p>
      </div>
      {related.length > 0 && (
        <div>
          <SectionTitle>Comes up with</SectionTitle>
          <div className="flex flex-wrap gap-1.5">
            {related.map(({ node: n, n: c }) => (
              <Chip key={n.id} n={n} onSelect={onSelect} count={c} />
            ))}
          </div>
        </div>
      )}
      <div>
        <SectionTitle>{isFog ? "Each time" : "Thoughts"}</SectionTitle>
        {thoughts.map((t) => (
          <div key={t.id}>
            <ThoughtLink n={t} onSelect={onSelect} />
            {isFog && (fogByThought.get(t.id) ?? []).length > 0 && (
              <div className="mb-2 ml-2">
                <FogList signals={fogByThought.get(t.id)!} />
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
