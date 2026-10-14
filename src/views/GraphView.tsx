// The mind map: every thought and entity as a node, linked by what Nodemind
// heard. Thoughts orbit the center; entities ring the outside in type sectors.

import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  Background,
  Controls,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import * as api from "../lib/api";
import type { GraphData, NodeView, NodeType, SearchHit } from "../lib/types";

const TYPE_COLOR: Record<NodeType, string> = {
  thought: "#4f6ef5",
  topic: "#7c5ce4",
  person: "#ec4899",
  goal: "#f59e0b",
  task: "#10b981",
  fog: "#64748b",
};

const TYPE_ORDER: NodeType[] = ["topic", "person", "goal", "task", "fog"];

// ── deterministic layout ────────────────────────────────────────────────────

interface Pos {
  x: number;
  y: number;
}

function layout(data: GraphData): Map<string, Pos> {
  const pos = new Map<string, Pos>();
  const thoughts = data.nodes.filter((n) => n.nodeType === "thought");
  const entities = data.nodes.filter((n) => n.nodeType !== "thought");

  const ring = (list: NodeView[], radius: number, a0: number, a1: number) => {
    if (list.length === 0) return;
    if (list.length === 1) {
      const a = (a0 + a1) / 2;
      pos.set(list[0].id, { x: Math.cos(a) * radius, y: Math.sin(a) * radius });
      return;
    }
    list.forEach((n, i) => {
      const a = a0 + ((i + 0.5) / list.length) * (a1 - a0);
      pos.set(n.id, { x: Math.cos(a) * radius, y: Math.sin(a) * radius });
    });
  };

  // Thoughts: inner ring, newest first (they arrive newest-first from the db).
  ring(thoughts, 200, 0, Math.PI * 2);

  // Entities: outer ring, one sector per type.
  const byType = new Map<NodeType, NodeView[]>();
  for (const n of entities) {
    const list = byType.get(n.nodeType) ?? [];
    list.push(n);
    byType.set(n.nodeType, list);
  }
  const present = TYPE_ORDER.filter((t) => byType.has(t));
  const total = entities.length || 1;
  let i = 0;
  for (const t of present) {
    const list = byType.get(t)!;
    ring(
      list,
      460,
      (i / total) * Math.PI * 2 - Math.PI / 2,
      ((i + list.length) / total) * Math.PI * 2 - Math.PI / 2,
    );
    i += list.length;
  }
  return pos;
}

// ── custom node ─────────────────────────────────────────────────────────────

interface MindNodeData extends Record<string, unknown> {
  node: NodeView;
  highlight: boolean;
}

type MindNode = Node<MindNodeData, "mind">;

function MindNodeComponent({ data }: NodeProps<MindNode>) {
  const { node, highlight } = data;
  const color = TYPE_COLOR[node.nodeType] ?? "#64748b";
  const scale = 1 + Math.min(0.5, node.degree * 0.04 + node.mentions * 0.02);
  return (
    <div
      className="flex max-w-44 items-center gap-1.5 rounded-full border px-2.5 py-1"
      style={{
        background: color + "1f",
        borderColor: color + (highlight ? "ff" : "66"),
        boxShadow: highlight ? `0 0 14px ${color}88` : undefined,
        transform: `scale(${scale})`,
      }}
    >
      <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: color }} />
      <span className="truncate text-[11px] font-medium text-ink" title={node.label}>
        {node.label}
      </span>
    </div>
  );
}

const nodeTypes: NodeTypes = { mind: MindNodeComponent };

// ── view ────────────────────────────────────────────────────────────────────

export default function GraphView({
  refreshKey,
  focusId = null,
}: {
  refreshKey: number;
  /** Thought to highlight on open (from Search → "Show in mind map"). */
  focusId?: string | null;
}) {
  const [data, setData] = useState<GraphData | null>(null);
  const [selected, setSelected] = useState<NodeView | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);
  const [highlightId, setHighlightId] = useState<string | null>(null);

  const load = () => {
    api
      .getGraph()
      .then((d) => {
        setData(d);
        setSelected((prev) =>
          prev ? d.nodes.find((n) => n.id === prev.id) ?? prev : prev,
        );
      })
      .catch(() => setData({ nodes: [], edges: [] }));
  };

  useEffect(load, [refreshKey]);

  // Arriving from Search: spotlight that thought once the graph is loaded.
  useEffect(() => {
    if (!focusId || !data) return;
    setHighlightId(focusId);
    setSelected((p) => data.nodes.find((n) => n.id === focusId) ?? p);
  }, [focusId, data]);

  // Refresh when the backend reports new thoughts/tasks (new edges may appear).
  useEffect(() => {
    const un1 = api.on<unknown>("thoughts-updated", load);
    const un2 = api.on<unknown>("tasks-updated", load);
    return () => {
      void un1.then((f) => f());
      void un2.then((f) => f());
    };
  }, [refreshKey]);

  const { rfNodes, rfEdges } = useMemo(() => {
    if (!data) return { rfNodes: [] as MindNode[], rfEdges: [] as Edge[] };
    const pos = layout(data);
    const rfNodes: MindNode[] = data.nodes
      .filter((n) => pos.has(n.id))
      .map((n) => ({
        id: n.id,
        type: "mind",
        position: pos.get(n.id)!,
        data: { node: n, highlight: highlightId === n.id },
      }));
    const idSet = new Set(data.nodes.map((n) => n.id));
    const rfEdges: Edge[] = data.edges
      .filter((e) => idSet.has(e.fromNode) && idSet.has(e.toNode))
      .map((e) => ({
        id: e.id,
        source: e.fromNode,
        target: e.toNode,
        label: e.weight > 1.5 ? String(Math.round(e.weight)) : undefined,
        style: { stroke: "var(--line-strong)", strokeWidth: Math.min(2.5, e.weight) },
      }));
    return { rfNodes, rfEdges };
  }, [data, highlightId]);

  const search = async () => {
    if (query.trim().length < 2) return;
    setSearching(true);
    try {
      const h = await api.searchMemory(query.trim(), 8);
      setHits(h);
      const top = h[0]?.thoughtId;
      if (top) {
        setHighlightId(top);
        setSelected((p) => data?.nodes.find((n) => n.id === top) ?? p);
      }
    } catch {
      setHits([]);
    } finally {
      setSearching(false);
    }
  };

  const onNodeClick = (_e: ReactMouseEvent, n: Node) => {
    setSelected((n as MindNode).data.node);
    setHighlightId(null);
  };

  return (
    <div className="relative flex flex-1 overflow-hidden">
      <div className="relative flex-1">
        <ReactFlow
          nodes={rfNodes}
          edges={rfEdges}
          nodeTypes={nodeTypes}
          onNodeClick={onNodeClick}
          onPaneClick={() => setSelected(null)}
          fitView
          minZoom={0.15}
          maxZoom={2}
          proOptions={{ hideAttribution: true }}
        >
          <Background color="var(--line)" gap={28} size={1} />
          <Controls className="!border-line !bg-surface !shadow-sm [&>button]:!border-line [&>button]:!bg-surface-2 [&>button]:!text-ink-2" />
        </ReactFlow>

        {(!data || data.nodes.length === 0) && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <p className="rounded-xl bg-surface px-6 py-4 text-sm text-ink-3 shadow-sm">
              Your map is empty — capture a brain-dump to plant the first nodes.
            </p>
          </div>
        )}

        {/* legend */}
        <div className="absolute bottom-3 left-3 flex gap-3 rounded-xl bg-surface/80 px-3 py-2 text-[10px] text-ink-3 backdrop-blur">
          {(["thought", "topic", "person", "goal"] as NodeType[]).map((t) => (
            <span key={t} className="flex items-center gap-1">
              <span className="h-2 w-2 rounded-full" style={{ background: TYPE_COLOR[t] }} />
              {t}
            </span>
          ))}
        </div>
      </div>

      {/* right panel: search + node detail */}
      <aside className="no-drag flex w-72 shrink-0 flex-col border-l border-line bg-surface-2">
        <div className="border-b border-line p-3">
          <div className="flex gap-2">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void search()}
              placeholder="Search your mind…"
              className="min-w-0 flex-1 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs outline-none placeholder:text-ink-3 focus:border-accent"
            />
            <button
              onClick={() => void search()}
              disabled={searching}
              className="rounded-lg bg-accent px-3 text-xs font-semibold text-white disabled:opacity-50"
            >
              {searching ? "…" : "Go"}
            </button>
          </div>
          {hits.length > 0 && (
            <div className="mt-2 space-y-1">
              {hits.map((h) => (
                <button
                  key={h.thoughtId}
                  onClick={() => {
                    setHighlightId(h.thoughtId);
                    setSelected((p) => data?.nodes.find((n) => n.id === h.thoughtId) ?? p);
                  }}
                  className="block w-full truncate rounded-lg px-2 py-1.5 text-left text-[11px] text-ink-2 hover:bg-surface"
                  title={h.snippet}
                >
                  {h.summary ?? h.snippet}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {selected ? (
            <NodeDetail node={selected} />
          ) : (
            <p className="text-xs leading-relaxed text-ink-3">
              Click a node to see what Nodemind remembers about it.
              <br />
              <br />
              Node size grows with how connected — and how often mentioned — it
              is.
            </p>
          )}
        </div>
      </aside>
    </div>
  );
}

function NodeDetail({ node }: { node: NodeView }) {
  const color = TYPE_COLOR[node.nodeType] ?? "#64748b";
  const when = node.lastSeen
    ? new Date(Number(node.lastSeen) * 1000).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      })
    : null;
  return (
    <div className="space-y-3 fade-up">
      <div className="flex items-center gap-2">
        <span className="h-2.5 w-2.5 rounded-full" style={{ background: color }} />
        <span className="text-[11px] font-semibold uppercase tracking-wide text-ink-3">
          {node.nodeType}
        </span>
        {when && <span className="ml-auto text-[10px] text-ink-3">seen {when}</span>}
      </div>
      <h3 className="text-sm font-bold leading-snug">{node.label}</h3>
      {node.excerpt && node.excerpt !== node.label && (
        <p className="text-xs leading-relaxed text-ink-2">{node.excerpt}</p>
      )}
      {node.summary && (
        <p className="rounded-lg bg-accent/8 px-3 py-2 text-xs text-ink">
          <span className="font-semibold text-accent">Summary: </span>
          {node.summary}
        </p>
      )}
      <div className="flex gap-3 pt-1 text-[11px] text-ink-3">
        <span>
          <b className="text-ink-2">{node.mentions}</b>{" "}
          {node.mentions === 1 ? "mention" : "mentions"}
        </span>
        <span>
          <b className="text-ink-2">{node.degree}</b>{" "}
          {node.degree === 1 ? "link" : "links"}
        </span>
        <span className="capitalize">{node.sentiment}</span>
      </div>
    </div>
  );
}
