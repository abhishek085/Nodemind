import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Task, Goal, GraphNode, GraphEdge } from "../types";

interface NodeData {
  id: string;
  label: string;
  type: "goal" | "task" | "meeting" | "person" | "center" | "topic" | "fog_pattern" | "project" | "domain" | "action_group";
  mentionsCount?: number;
  parentId?: string;
  constituents?: string[];
  x: number;
  y: number;
  weight?: number;
  recency?: number;
  isShadow?: boolean;
  isGhost?: boolean;
  category?: "Vision" | "Execution" | "Network" | "Personal Battery" | "Parking Lot";
  impact?: "resolved" | "stressor" | "spark";
  signalType?: "intent" | "action";
  createdAt?: string;
  detail?: string;
}

interface NodeMeta {
  category?: "Vision" | "Execution" | "Network" | "Personal Battery" | "Parking Lot";
  status?: string;
  impact?: "resolved" | "stressor" | "spark";
  signal_type?: "intent" | "action";
  context?: string;
  mentions_count?: number;
}

export default function MentalMapView() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [goals, setGoals] = useState<Goal[]>([]);
  const [graphNodes, setGraphNodes] = useState<GraphNode[]>([]);
  const [graphEdges, setGraphEdges] = useState<GraphEdge[]>([]);
  const [selected, setSelected] = useState<NodeData | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<GraphEdge | null>(null);
  const [updatingMap, setUpdatingMap] = useState(false);
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());

  const fetchMapData = async () => {
    try {
      const [t, g, nodes, edges] = await Promise.all([
        invoke<Task[]>("get_tasks"),
        invoke<Goal[]>("get_goals"),
        invoke<GraphNode[]>("get_graph_nodes"),
        invoke<GraphEdge[]>("get_graph_edges"),
      ]);
      setTasks(t);
      setGoals(g);
      setGraphNodes(nodes);
      setGraphEdges(edges);
    } catch {}
  };

  const handleUpdateMap = async () => {
    setUpdatingMap(true);
    setUpdateStatus(null);
    try {
      const msg: string = await invoke("refresh_mental_map");
      setUpdateStatus(msg);
      await fetchMapData();
    } catch (err) {
      setUpdateStatus(`Update failed: ${err instanceof Error ? err.message : String(err ?? "Ollama not reachable")}`);
    } finally {
      setUpdatingMap(false);
      setTimeout(() => setUpdateStatus(null), 4000);
    }
  };

  useEffect(() => {
    fetchMapData();
  }, []);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    const setup = async () => {
      unsubs.push(await listen("mental-map-updated", fetchMapData));
      unsubs.push(await listen("live-notes-updated", fetchMapData));
      unsubs.push(await listen("tasks-updated", fetchMapData));
    };
    setup();
    return () => {
      unsubs.forEach((fn) => fn());
    };
  }, []);

  const parseNodeMeta = (raw?: string | null): NodeMeta => {
    if (!raw) return {};
    try {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      const categoryRaw = String(parsed.category ?? "").trim().toLowerCase();
      const category =
        categoryRaw === "vision"
          ? "Vision"
          : categoryRaw === "execution"
            ? "Execution"
            : categoryRaw === "network"
              ? "Network"
              : categoryRaw === "personal battery"
                ? "Personal Battery"
                : categoryRaw === "parking lot"
                  ? "Parking Lot"
                  : undefined;
      const impactRaw = String(parsed.impact ?? "").trim().toLowerCase();
      const impact =
        impactRaw === "resolved"
          ? "resolved"
          : impactRaw === "stressor"
            ? "stressor"
            : impactRaw === "spark"
              ? "spark"
              : undefined;
      const signalRaw = String(parsed.signal_type ?? "").trim().toLowerCase();
      const signalType = signalRaw === "intent" || signalRaw === "action" ? (signalRaw as "intent" | "action") : undefined;
      const rawMc = parsed.mentions_count;
      const mentions_count = typeof rawMc === "number"
        ? rawMc
        : typeof rawMc === "string" ? (parseInt(rawMc, 10) || undefined) : undefined;
      return {
        category,
        status: typeof parsed.status === "string" ? parsed.status : undefined,
        impact,
        signal_type: signalType,
        context: typeof parsed.context === "string" ? parsed.context : undefined,
        mentions_count,
      };
    } catch {
      return {};
    }
  };

  // Three-tier tree: Tier1=Domain anchors, Tier2=Topic/Project branches, Tier3=Task/Goal atoms
  const buildNodes = (): NodeData[] => {
    const cx = 340;
    const cy = 280;
    type DomainName = "Vision" | "Execution" | "Network" | "Personal Battery" | "Parking Lot";
    const DOMAINS: DomainName[] = ["Vision", "Execution", "Network", "Personal Battery", "Parking Lot"];
    const DOMAIN_ANGLES: Record<DomainName, number> = {
      Vision:            -3 * Math.PI / 4,
      Execution:         -Math.PI / 4,
      Network:            3 * Math.PI / 4,
      "Personal Battery": Math.PI / 4,
      "Parking Lot":      Math.PI / 2,
    };
    const SECTOR = (2 * Math.PI) / DOMAINS.length;

    // Fan `count` items evenly around `base` +/- `half` radians
    const fanAngles = (count: number, base: number, half: number): number[] =>
      count === 0 ? [] : count === 1 ? [base]
        : Array.from({ length: count }, (_, i) => base - half + (i / (count - 1)) * half * 2);

    const nodes: NodeData[] = [
      { id: "you", label: "You", type: "center", x: cx, y: cy },
    ];

    // Tier 1: virtual domain anchor nodes (always shown)
    DOMAINS.forEach((domain) => {
      const angle = DOMAIN_ANGLES[domain];
      nodes.push({
        id: `domain-${domain}`,
        label: domain,
        type: "domain",
        x: cx + 118 * Math.cos(angle),
        y: cy + 118 * Math.sin(angle),
        category: domain === "Parking Lot" ? undefined : (domain as NodeData["category"]),
      });
    });

    if (graphNodes.length === 0) {
      const activeGoals = goals.filter((g) => g.status === "active").slice(0, 6);
      const pendingTasks = tasks.filter((t) => !t.done).slice(0, 8);
      const wAngle = DOMAIN_ANGLES["Execution"];
      const fh = SECTOR * 0.35;
      activeGoals.forEach((g, i) => {
        const angs = fanAngles(activeGoals.length, wAngle - 0.1, fh);
        nodes.push({ id: `goal-${g.id}`, label: g.title.slice(0, 22), type: "goal",
          x: cx + (200 + i * 14) * Math.cos(angs[i] ?? wAngle),
          y: cy + (200 + i * 14) * Math.sin(angs[i] ?? wAngle),
          parentId: "domain-Execution", detail: g.description ?? undefined });
      });
      pendingTasks.slice(0, 5).forEach((t, i) => {
        const angs = fanAngles(pendingTasks.length, wAngle + 0.1, fh);
        nodes.push({ id: `task-${t.id}`, label: t.title.slice(0, 22), type: "task",
          x: cx + (215 + i * 14) * Math.cos(angs[i] ?? wAngle),
          y: cy + (215 + i * 14) * Math.sin(angs[i] ?? wAngle),
          parentId: "domain-Execution", detail: t.project ?? undefined });
      });
      return nodes;
    }

    // Ghost goal detection (today only)
    const goalIds = new Set(graphNodes.filter((n) => n.node_type === "goal").map((n) => n.id));
    const taskIds = new Set(graphNodes.filter((n) => n.node_type === "task").map((n) => n.id));
    const today = new Date().toDateString();
    const taskLinkedGoalIds = new Set<string>();
    for (const edge of graphEdges) {
      const srcAt = graphNodes.find((n) => n.id === edge.to_node)?.created_at;
      if (srcAt && new Date(srcAt).toDateString() !== today) continue;
      if (goalIds.has(edge.from_node) && taskIds.has(edge.to_node)) taskLinkedGoalIds.add(edge.from_node);
      if (goalIds.has(edge.to_node) && taskIds.has(edge.from_node)) taskLinkedGoalIds.add(edge.to_node);
    }

    // Classify graph nodes by domain
    const byDomain: Record<DomainName, GraphNode[]> = {
      Vision: [], Execution: [], Network: [], "Personal Battery": [], "Parking Lot": [],
    };
    graphNodes.forEach((gn) => {
      const meta = parseNodeMeta(gn.data);
      const cat = meta.category as DomainName | undefined;
      const bucket: DomainName = cat && (DOMAINS as readonly string[]).includes(cat) ? cat : "Parking Lot";
      byDomain[bucket].push(gn);
    });

    const tier2Types = new Set(["topic", "project"]);
    const tier3Types = new Set(["task", "goal", "person", "fog_pattern"]);

    DOMAINS.forEach((domain) => {
      const domainNodes = byDomain[domain];
      const tier2 = domainNodes.filter((n) => tier2Types.has(n.node_type));
      const tier3 = domainNodes.filter((n) => tier3Types.has(n.node_type));
      const sectorAnchor = DOMAIN_ANGLES[domain];
      const fanHalf = Math.min(SECTOR * 0.44, Math.PI / 3);

      // Build topic->atom assignment via graph edges then context fallback
      const topicAtoms: Record<string, string[]> = {};
      const atomClaimed = new Set<string>();
      tier2.forEach((t) => { topicAtoms[t.id] = []; });
      graphEdges.forEach((e) => {
        const fromT2 = tier2.find((n) => n.id === e.from_node);
        const toT3 = tier3.find((n) => n.id === e.to_node);
        if (fromT2 && toT3 && !atomClaimed.has(toT3.id)) {
          topicAtoms[fromT2.id].push(toT3.id); atomClaimed.add(toT3.id);
        }
        const toT2 = tier2.find((n) => n.id === e.to_node);
        const fromT3 = tier3.find((n) => n.id === e.from_node);
        if (toT2 && fromT3 && !atomClaimed.has(fromT3.id)) {
          topicAtoms[toT2.id].push(fromT3.id); atomClaimed.add(fromT3.id);
        }
      });
      tier3.forEach((atom) => {
        if (atomClaimed.has(atom.id)) return;
        const ctx = parseNodeMeta(atom.data).context?.toLowerCase().trim();
        if (!ctx) return;
        const matched = tier2.find((t) => {
          const tl = t.label.toLowerCase();
          return tl.includes(ctx) || ctx.includes(tl);
        });
        if (matched) { topicAtoms[matched.id].push(atom.id); atomClaimed.add(atom.id); }
      });
      const orphans = tier3.filter((a) => !atomClaimed.has(a.id));

      // Place Tier 2 (topic/project branch) nodes
      const t2Angles = fanAngles(tier2.length, sectorAnchor, tier2.length > 1 ? fanHalf : 0);
      tier2.forEach((t2node, i) => {
        const t2angle = t2Angles[i];
        const meta = parseNodeMeta(t2node.data);
        const mc = meta.mentions_count ?? 1;
        const t2x = cx + 230 * Math.cos(t2angle);
        const t2y = cy + 230 * Math.sin(t2angle);
        nodes.push({
          id: t2node.id,
          label: t2node.label.slice(0, 22),
          type: t2node.node_type as NodeData["type"],
          x: t2x, y: t2y,
          weight: Math.max(1, mc),
          recency: 0.9,
          category: domain === "Parking Lot" ? undefined : (domain as NodeData["category"]),
          mentionsCount: mc,
          parentId: `domain-${domain}`,
          isShadow: meta.status === "shadow",
          detail: meta.context,
        });

        // Place Tier 3 atoms under this topic
        const atoms = (topicAtoms[t2node.id] ?? [])
          .map((id) => tier3.find((n) => n.id === id)).filter(Boolean) as GraphNode[];
        const COLLAPSE_THRESHOLD = 5;
        const isGrouped = atoms.length > COLLAPSE_THRESHOLD && !expandedGroups.has(t2node.id);

        if (isGrouped) {
          nodes.push({
            id: `group-${t2node.id}`,
            label: `${atoms.length} actions`,
            type: "action_group",
            x: t2x + 100 * Math.cos(t2angle),
            y: t2y + 100 * Math.sin(t2angle),
            category: domain === "Parking Lot" ? undefined : (domain as NodeData["category"]),
            parentId: t2node.id,
            constituents: atoms.map((a) => a.id),
          });
        } else {
          const atomHalf = Math.min(0.48, 0.08 + atoms.length * 0.07);
          const atomAngles = fanAngles(atoms.length, t2angle, atomHalf);
          atoms.forEach((atom, j) => {
            const aAngle = atomAngles[j];
            const aMeta = parseNodeMeta(atom.data);
            const mc2 = aMeta.mentions_count ?? 1;
            nodes.push({
              id: atom.id,
              label: atom.label.slice(0, 22),
              type: atom.node_type as NodeData["type"],
              x: t2x + (88 + j * 18) * Math.cos(aAngle),
              y: t2y + (88 + j * 18) * Math.sin(aAngle),
              weight: Math.max(1, mc2), recency: 0.65,
              category: domain === "Parking Lot" ? undefined : (domain as NodeData["category"]),
              impact: aMeta.impact, signalType: aMeta.signal_type,
              mentionsCount: mc2 > 1 ? mc2 : undefined,
              parentId: t2node.id,
              isShadow: aMeta.status === "shadow",
              isGhost: atom.node_type === "goal" && !taskLinkedGoalIds.has(atom.id),
              detail: aMeta.context,
            });
          });
        }
      });

      // Orphan atoms placed directly under domain
      const orphanHalf = Math.min(fanHalf * 0.8, 0.08 + orphans.length * 0.08);
      const orphanAngles = fanAngles(orphans.length, sectorAnchor, orphanHalf);
      orphans.forEach((atom, i) => {
        const aAngle = orphanAngles[i];
        const aMeta = parseNodeMeta(atom.data);
        const mc = aMeta.mentions_count ?? 1;
        nodes.push({
          id: atom.id, label: atom.label.slice(0, 22),
          type: atom.node_type as NodeData["type"],
          x: cx + (195 + i * 15) * Math.cos(aAngle),
          y: cy + (195 + i * 15) * Math.sin(aAngle),
          weight: Math.max(1, mc), recency: 0.6,
          category: domain === "Parking Lot" ? undefined : (domain as NodeData["category"]),
          impact: aMeta.impact, signalType: aMeta.signal_type,
          mentionsCount: mc > 1 ? mc : undefined,
          parentId: `domain-${domain}`,
          isShadow: aMeta.status === "shadow",
          isGhost: atom.node_type === "goal" && !taskLinkedGoalIds.has(atom.id),
          detail: aMeta.context,
        });
      });
    });

    return nodes;
  };
  // Lightweight force pass: sibling nodes repel so branches stay distributed.
  const applyRepulsion = (seed: NodeData[]): NodeData[] => {
    const movable = seed.map((n) => ({ ...n }));
    const fixedIds = new Set(movable.filter((n) => n.type === "center" || n.type === "domain").map((n) => n.id));
    for (let step = 0; step < 24; step++) {
      for (let i = 0; i < movable.length; i++) {
        const a = movable[i];
        if (fixedIds.has(a.id)) continue;
        let fx = 0;
        let fy = 0;
        for (let j = 0; j < movable.length; j++) {
          if (i === j) continue;
          const b = movable[j];
          const dx = a.x - b.x;
          const dy = a.y - b.y;
          const dist2 = Math.max(64, dx * dx + dy * dy);
          const force = 2600 / dist2;
          fx += (dx / Math.sqrt(dist2)) * force;
          fy += (dy / Math.sqrt(dist2)) * force;
        }
        a.x = Math.max(36, Math.min(644, a.x + fx));
        a.y = Math.max(36, Math.min(524, a.y + fy));
      }
    }
    return movable;
  };

  const nodes = applyRepulsion(buildNodes());

  const treeEdges = nodes
    .filter((n) => n.parentId)
    .map((n) => {
      const from = nodes.find((m) => m.id === n.parentId);
      if (!from) return null;
      const isDomainToTopic = from.type === "domain" && (n.type === "topic" || n.type === "project");
      const strokeWidth = isDomainToTopic
        ? Math.max(1.5, Math.min(8, 1 + (n.mentionsCount ?? 1) * 0.7))
        : (n.type === "action_group" ? 1.5 : 1.2);
      return {
        id: `${from.id}->${n.id}`,
        from,
        to: n,
        strokeWidth,
        strokeOpacity: isDomainToTopic ? 0.55 : 0.35,
      };
    })
    .filter(Boolean) as Array<{
      id: string;
      from: NodeData;
      to: NodeData;
      strokeWidth: number;
      strokeOpacity: number;
    }>;

  const bucketColors: Record<"Vision" | "Execution" | "Network" | "Personal Battery" | "Parking Lot", string> = {
    Vision:            "#7c3aed",
    Execution:         "#2563eb",
    Network:           "#f59e0b",
    "Personal Battery": "#10b981",
    "Parking Lot":      "#64748b",
  };

  const typeFallbackColors: Record<NodeData["type"], string> = {
    center: "#6366f1",
    goal: "#f59e0b",
    task: "#10b981",
    meeting: "#3b82f6",
    person: "#ec4899",
    topic: "#8b5cf6",
    project: "#06b6d4",
    fog_pattern: "#ef4444",
    domain: "#475569",
    action_group: "#94a3b8",
  };

  const colorForNode = (n: NodeData): string => {
    if (n.impact === "resolved") return "#22c55e";
    if (n.impact === "stressor") return "#ef4444";
    if (n.category) return bucketColors[n.category];
    return typeFallbackColors[n.type];
  };

  return (
    <div className="view-map">
      <div className="view-header">
        <div>
          <h1 className="view-title">Mental Map</h1>
          <p className="view-subtitle">Domain → Topic → Atoms tree with branch fattening</p>
        </div>
        <div className="header-actions" style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
          <button
            className="action-btn"
            onClick={handleUpdateMap}
            disabled={updatingMap}
            title="Re-process unread notes and refresh the mental map"
          >
            {updatingMap ? "Updating..." : "Update Mental Map"}
          </button>
          <div className="map-legend">
            {Object.entries(bucketColors)
              .map(([type, color]) => (
                <span key={type} className="legend-item">
                  <span className="legend-dot" style={{ background: color }} />
                  {type}
                </span>
              ))}
          </div>
        </div>
      </div>

      {updateStatus && (
        <div className="card feedback-card" style={{ marginBottom: "0.5rem" }}>
          <p className="feedback-text">{updateStatus}</p>
        </div>
      )}

      <div className="map-layout">
        <div className="map-canvas-wrap">
          <svg viewBox="0 0 680 560" className="map-svg" preserveAspectRatio="xMidYMid meet">
            {/* Tier guides */}
            <circle cx={340} cy={280} r={118} fill="none" stroke="#94a3b8" strokeOpacity={0.35} strokeDasharray="4,6" />
            <circle cx={340} cy={280} r={230} fill="none" stroke="#94a3b8" strokeOpacity={0.28} strokeDasharray="4,8" />
            <circle cx={340} cy={280} r={320} fill="none" stroke="#94a3b8" strokeOpacity={0.20} strokeDasharray="3,9" />
            <text x={340} y={154} textAnchor="middle" fontSize={10} fill="#94a3b8" fillOpacity={0.75}>Tier 1: Domain</text>
            <text x={340} y={48} textAnchor="middle" fontSize={10} fill="#94a3b8" fillOpacity={0.65}>Tier 2: Topic</text>
            <text x={340} y={10} textAnchor="middle" fontSize={10} fill="#94a3b8" fillOpacity={0.60}>Tier 3: Atoms</text>

            {/* Tree edges (parent -> child) */}
            {treeEdges.map((edge) => (
              <line
                key={edge.id}
                x1={edge.from.x}
                y1={edge.from.y}
                x2={edge.to.x}
                y2={edge.to.y}
                stroke={colorForNode(edge.to)}
                strokeWidth={edge.strokeWidth}
                strokeOpacity={edge.strokeOpacity}
              />
            ))}

            {/* Causal links from extracted graph edges */}
            {graphEdges.map((edge) => {
              const fromNode = nodes.find((n) => n.id === edge.from_node);
              const toNode = nodes.find((n) => n.id === edge.to_node);
              if (!fromNode || !toNode) return null;
              const isSelected = selectedEdge?.id === edge.id;
              const isTrigger = edge.edge_type === "triggers";
              return (
                <line
                  key={`graph-edge-${edge.id}`}
                  x1={fromNode.x}
                  y1={fromNode.y}
                  x2={toNode.x}
                  y2={toNode.y}
                  stroke={isSelected ? "#111827" : "#9ca3af"}
                  strokeWidth={isSelected ? 2.6 : 1.2}
                  strokeOpacity={isSelected ? 0.95 : 0.35}
                  strokeDasharray="5,5"
                  markerEnd={isTrigger ? "url(#arrowhead)" : undefined}
                  style={{ cursor: "pointer" }}
                  onClick={() => {
                    setSelectedEdge(isSelected ? null : edge);
                    setSelected(null);
                  }}
                />
              );
            })}

            {/* Nodes */}
            {nodes.map((n) => {
              const r = n.type === "center" ? 26 : n.type === "domain" ? 18 : n.type === "action_group" ? 14 : 16;
              const color = colorForNode(n);
              const isSelected = selected?.id === n.id;
              return (
                <g
                  key={n.id}
                  className="map-node"
                  onClick={() => {
                    if (n.type === "action_group" && n.parentId) {
                      const next = new Set(expandedGroups);
                      if (next.has(n.parentId)) next.delete(n.parentId);
                      else next.add(n.parentId);
                      setExpandedGroups(next);
                      setSelected(null);
                      setSelectedEdge(null);
                      return;
                    }
                    setSelected(isSelected ? null : n);
                    setSelectedEdge(null);
                  }}
                  style={{ cursor: "pointer" }}
                >
                  <circle
                    cx={n.x}
                    cy={n.y}
                    r={r + Math.min(7, Math.floor((n.weight ?? 1) * 0.8)) + (isSelected ? 4 : 0)}
                    fill={color}
                    fillOpacity={n.isShadow ? 0.35 : (isSelected ? 1 : 0.45 + (n.recency ?? 0.75) * 0.5)}
                    stroke={n.isGhost ? "#6b7280" : (n.isShadow ? "#999" : "white")}
                    strokeWidth={isSelected ? 3 : 1.5}
                    strokeDasharray={n.isGhost || n.isShadow ? "4,2" : undefined}
                    filter={isSelected ? "url(#glow)" : undefined}
                  />
                  {n.impact === "spark" && (
                    <text
                      x={n.x + r - 2}
                      y={n.y - r + 4}
                      textAnchor="middle"
                      fontSize={13}
                      className="node-spark"
                    >
                      ★
                    </text>
                  )}
                  {(n.mentionsCount ?? 0) > 1 && (
                    <text
                      x={n.x + r + 2}
                      y={n.y - r + 2}
                      textAnchor="middle"
                      fontSize={10}
                      fill="#f59e0b"
                      style={{ fontWeight: 700 }}
                    >
                      +{n.mentionsCount}
                    </text>
                  )}
                  <text
                    x={n.x}
                    y={n.y + r + 14}
                    textAnchor="middle"
                    fontSize={10}
                    fill="currentColor"
                    className="node-label"
                  >
                    {n.label}
                  </text>
                </g>
              );
            })}

            <defs>
              <filter id="glow" x="-30%" y="-30%" width="160%" height="160%">
                <feGaussianBlur stdDeviation="4" result="blur" />
                <feMerge>
                  <feMergeNode in="blur" />
                  <feMergeNode in="SourceGraphic" />
                </feMerge>
              </filter>
              <marker id="arrowhead" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
                <polygon points="0 0, 8 4, 0 8" fill="#6b7280" />
              </marker>
            </defs>
          </svg>
        </div>

        {/* Detail panel */}
        {selected && selected.id !== "you" && (
          <div className="map-detail card">
            <div className="map-detail-type">{selected.type}</div>
            <h3 className="map-detail-title">{selected.label}</h3>
            <p className="hint-text">
              {selected.category ? `Domain: ${selected.category}` : "Domain: Parking Lot"}
              {selected.signalType ? ` • ${selected.signalType === "intent" ? "Intent" : "Action"}` : ""}
              {selected.impact ? ` • ${selected.impact}` : ""}
              {selected.mentionsCount ? ` • mentions: ${selected.mentionsCount}` : ""}
            </p>
            {selected.detail && (
              <p className="map-detail-desc">{selected.detail}</p>
            )}
            <p className="hint-text mt-1">
              Recency: {Math.round((selected.recency ?? 0) * 100)}% • Mental gravity: {Math.round(selected.weight ?? 1)}
            </p>
            {selected.isGhost && (
              <p className="hint-text mt-1">Ghost goal: no linked action node yet today.</p>
            )}
            {selected.type === "goal" && (
              <div className="map-related">
                <div className="section-label">Related Tasks</div>
                {tasks
                  .filter(
                    (t) =>
                      !t.done &&
                      t.project &&
                      selected.label.toLowerCase().includes(t.project.toLowerCase()),
                  )
                  .slice(0, 4)
                  .map((t) => (
                    <div key={t.id} className="related-item">
                      — {t.title}
                    </div>
                  ))}
              </div>
            )}
            {selected.type === "action_group" && (
              <p className="hint-text mt-1">Click this group bubble to expand or collapse its action leaves.</p>
            )}
          </div>
        )}

        {selectedEdge && (
          <div className="map-detail card">
            <div className="map-detail-type">connection</div>
            <h3 className="map-detail-title">Why these are linked</h3>
            <p className="map-detail-desc">
              Relation: {selectedEdge.edge_type} • Strength: {selectedEdge.weight.toFixed(2)}
            </p>
            <p className="hint-text mt-1">
              Evidence from 5-minute summaries is not persisted yet. Next step is attaching the originating live-note ID when writing each edge.
            </p>
          </div>
        )}

        {nodes.length <= 1 && (
          <div className="card map-empty">
            <p>Your mental map will fill in as you start listening.</p>
            <p className="hint-text mt-1">
              Goals, tasks, meetings, and people will appear as bubbles.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
