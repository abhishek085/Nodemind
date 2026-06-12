import React, { useEffect, useRef, useState, useCallback } from "react";
import type { GraphNode, GraphEdge, GraphLens } from "../types";
import GraphLensBar from "../components/GraphLensBar";

// ─── Constants ──────────────────────────────────────────────────────────────
const REPULSION = 800;
const ATTRACTION_BASE = 0.03;
const CENTER_GRAVITY = 0.002;
const MAX_ITERATIONS = 300;
const VELOCITY_THRESHOLD = 0.5;
const BASE_RADIUS = 8;
const MAX_RADIUS = 24;

const NODE_COLORS: Record<string, string> = {
  project: "#38BDF8",
  goal:    "#34D399",
  person:  "#FBBF24",
  task:    "#38BDF8",
  topic:   "#818CF8",
  fog_pattern: "#F87171",
  fog:     "#F87171",
  idea:    "#818CF8",
  self:    "#7B5CF6",
};

// ─── Types ────────────────────────────────────────────────────────────────────
interface SimNode {
  id: string;
  label: string;
  node_type: string;
  mentions_count: number;
  last_mentioned_at?: string | null;
  sentiment: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  pinned: boolean;
  radius: number;
}

interface SimEdge {
  from: string;
  to: string;
  weight: number;
  particleOffset: number; // 0–1 along the edge
}

interface KnowledgeGraphViewProps {
  nodes: GraphNode[];
  edges: GraphEdge[];
  momentumScores: Record<string, number>;
  onNavigateToNode?: (id: string) => void;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────
function nodeRadius(mentions: number): number {
  return Math.min(BASE_RADIUS + Math.log(mentions + 1) * 4, MAX_RADIUS);
}

function applyLensFilter(
  nodes: GraphNode[],
  edges: GraphEdge[],
  lens: GraphLens
): { nodes: GraphNode[]; edges: GraphEdge[] } {
  if (lens === "memory") return { nodes, edges };

  if (lens === "tasks") {
    const filtered = nodes.filter((n) => n.node_type === "task" || n.node_type === "project");
    const ids = new Set(filtered.map((n) => n.id));
    return { nodes: filtered, edges: edges.filter((e) => ids.has(e.from_node) && ids.has(e.to_node)) };
  }

  // For goals, week — always include the self node so the graph has its anchor
  const selfNodes = nodes.filter((n) => n.node_type === "self");

  if (lens === "goals") {
    const filtered = [...selfNodes, ...nodes.filter((n) => n.node_type === "goal" || n.node_type === "project")];
    const ids = new Set(filtered.map((n) => n.id));
    return { nodes: filtered, edges: edges.filter((e) => ids.has(e.from_node) && ids.has(e.to_node)) };
  }

  if (lens === "week") {
    const cutoff = Date.now() - 7 * 86400000;
    const filtered = [...selfNodes, ...nodes.filter((n) => {
      if (n.node_type === "self") return false; // already included above
      const ts = n.last_mentioned_at ? new Date(n.last_mentioned_at).getTime() : new Date(n.created_at).getTime();
      return ts >= cutoff;
    })];
    const ids = new Set(filtered.map((n) => n.id));
    return { nodes: filtered, edges: edges.filter((e) => ids.has(e.from_node) && ids.has(e.to_node)) };
  }

  return { nodes, edges };
}

// ─── Force simulation step ───────────────────────────────────────────────────
function runForceStep(simNodes: SimNode[], simEdges: SimEdge[]): number {
  // 1. Repulsion
  for (let i = 0; i < simNodes.length; i++) {
    for (let j = i + 1; j < simNodes.length; j++) {
      const a = simNodes[i];
      const b = simNodes[j];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist2 = dx * dx + dy * dy + 0.1;
      const force = REPULSION / dist2;
      const fx = (dx / Math.sqrt(dist2)) * force;
      const fy = (dy / Math.sqrt(dist2)) * force;
      if (!a.pinned) { a.vx -= fx; a.vy -= fy; }
      if (!b.pinned) { b.vx += fx; b.vy += fy; }
    }
  }

  // 2. Attraction along edges
  const nodeMap = new Map<string, SimNode>(simNodes.map((n) => [n.id, n]));
  for (const edge of simEdges) {
    const a = nodeMap.get(edge.from);
    const b = nodeMap.get(edge.to);
    if (!a || !b) continue;
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const strength = ATTRACTION_BASE * Math.min(edge.weight, 2.0);
    const fx = dx * strength;
    const fy = dy * strength;
    if (!a.pinned) { a.vx += fx; a.vy += fy; }
    if (!b.pinned) { b.vx -= fx; b.vy -= fy; }
  }

  // 3. Center gravity
  for (const n of simNodes) {
    if (!n.pinned) {
      n.vx += (0 - n.x) * CENTER_GRAVITY;
      n.vy += (0 - n.y) * CENTER_GRAVITY;
    }
  }

  // 4. Apply + dampen
  let maxV = 0;
  for (const n of simNodes) {
    if (n.pinned) continue;
    n.vx *= 0.85;
    n.vy *= 0.85;
    n.x += n.vx;
    n.y += n.vy;
    const v = Math.sqrt(n.vx * n.vx + n.vy * n.vy);
    if (v > maxV) maxV = v;
  }
  return maxV;
}

// ─── Main component ───────────────────────────────────────────────────────────
const KnowledgeGraphView: React.FC<KnowledgeGraphViewProps> = ({
  nodes,
  edges,
  momentumScores,
  onNavigateToNode: _onNavigateToNode,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [lens, setLens] = useState<GraphLens>("memory");
  const [selectedNode, setSelectedNode] = useState<GraphNode | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null); // double-click focus

  // Camera state
  const cameraRef = useRef({ x: 0, y: 0, zoom: 1 });
  const isDraggingCanvas = useRef(false);
  const canvasDragStart = useRef({ mx: 0, my: 0, cx: 0, cy: 0 });

  // Sim state persisted via ref for rAF loop
  const simNodesRef = useRef<SimNode[]>([]);
  const simEdgesRef = useRef<SimEdge[]>([]);
  const iterRef = useRef(0);
  const frozenRef = useRef(false);
  const rafRef = useRef<number>(0);
  const draggingNodeRef = useRef<SimNode | null>(null);

  // Build/rebuild sim when node data or lens changes
  useEffect(() => {
    const { nodes: filteredNodes, edges: filteredEdges } = applyLensFilter(nodes, edges, lens);

    const canvas = canvasRef.current;
    const w = canvas?.width ?? 800;
    const h = canvas?.height ?? 600;
    const cx = w / 2;
    const cy = h / 2;

    // Preserve positions for nodes that already exist in sim
    const prevMap = new Map<string, SimNode>(simNodesRef.current.map((n) => [n.id, n]));

    // Ensure the "You" self node is always present in the sim.
    // If the backend hasn't seeded it yet (fresh install / empty DB), inject it
    // as a virtual node so the graph always has a centre anchor.
    const hasself = filteredNodes.some((n) => n.node_type === "self");
    const allNodes: GraphNode[] = hasself ? filteredNodes : [
      {
        id: "self-you",
        node_type: "self",
        label: "You",
        data: "{}",
        created_at: new Date().toISOString(),
        mentions_count: 1,
        last_mentioned_at: null,
        sentiment: "neutral",
      } as GraphNode,
      ...filteredNodes,
    ];

    simNodesRef.current = allNodes.map((n) => {
      const prev = prevMap.get(n.id);
      const isSelf = n.node_type === "self";
      const r = isSelf ? MAX_RADIUS : nodeRadius(n.mentions_count ?? 1);
      return {
        id: n.id,
        label: n.label,
        node_type: n.node_type,
        mentions_count: n.mentions_count ?? 1,
        last_mentioned_at: n.last_mentioned_at,
        sentiment: n.sentiment,
        // Self node is always pinned at sim origin (0,0) — canvas translates to centre
        x: isSelf ? 0 : (prev ? prev.x : (Math.random() - 0.5) * 300),
        y: isSelf ? 0 : (prev ? prev.y : (Math.random() - 0.5) * 300),
        vx: 0,
        vy: 0,
        pinned: isSelf ? true : (prev ? prev.pinned : false),
        radius: r,
      };
    });

    const maxW = Math.max(...filteredEdges.map((e) => e.weight), 0.1);
    simEdgesRef.current = filteredEdges.map((e) => ({
      from: e.from_node,
      to: e.to_node,
      weight: e.weight / maxW,
      particleOffset: Math.random(),
    }));

    iterRef.current = 0;
    frozenRef.current = false;

    // Reset camera
    cameraRef.current = { x: cx, y: cy, zoom: 1 };
  }, [nodes, edges, lens]);

  // Star field (initialized once)
  const starsRef = useRef<{ x: number; y: number; r: number; a: number }[]>([]);
  const [showLabels, setShowLabels] = useState(true);

  // Draw loop
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let lastTime = 0;

    const draw = (timestamp: number) => {
      rafRef.current = requestAnimationFrame(draw);
      const dt = timestamp - lastTime;
      if (dt < 14) return;
      lastTime = timestamp;

      const dpr = window.devicePixelRatio || 1;
      const W = canvas.width / dpr;
      const H = canvas.height / dpr;
      const cam = cameraRef.current;

      ctx.save();
      ctx.scale(dpr, dpr);

      // Background
      ctx.fillStyle = "#09090f";
      ctx.fillRect(0, 0, W, H);

      // Star field
      if (starsRef.current.length === 0) {
        starsRef.current = Array.from({ length: 70 }, () => ({
          x: Math.random() * W,
          y: Math.random() * H,
          r: Math.random() * 0.8 + 0.2,
          a: Math.random(),
        }));
      }
      starsRef.current.forEach((s) => {
        ctx.globalAlpha = s.a * 0.4;
        ctx.fillStyle = "#ffffff";
        ctx.beginPath();
        ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
        ctx.fill();
      });
      ctx.globalAlpha = 1;

      ctx.translate(cam.x, cam.y);
      ctx.scale(cam.zoom, cam.zoom);

      const t = timestamp;
      const simNodes = simNodesRef.current;
      const simEdges = simEdgesRef.current;

      // Run simulation step if not frozen
      if (!frozenRef.current) {
        const maxV = runForceStep(simNodes, simEdges);
        iterRef.current++;
        if (iterRef.current >= MAX_ITERATIONS || maxV < VELOCITY_THRESHOLD) {
          frozenRef.current = true;
        }
      }

      // Determine focus set
      const focusNeighbors: Set<string> | null = focusId
        ? new Set(
            simEdges
              .filter((e) => e.from === focusId || e.to === focusId)
              .flatMap((e) => [e.from, e.to])
          )
        : null;

      const selectedId = selectedNode?.id ?? null;

      // ── Draw edges (quadratic bezier curves) ──────────────────────────────
      for (const edge of simEdges) {
        const a = simNodes.find((n) => n.id === edge.from);
        const b = simNodes.find((n) => n.id === edge.to);
        if (!a || !b) continue;

        const isConnectedToHovered = hoveredId && (edge.from === hoveredId || edge.to === hoveredId);
        const isConnectedToSelected = selectedId && (edge.from === selectedId || edge.to === selectedId);
        const isInFocus = !focusNeighbors || (focusNeighbors.has(edge.from) && focusNeighbors.has(edge.to));

        let edgeAlpha: number;
        if (focusNeighbors) {
          edgeAlpha = isInFocus ? 0.55 : 0.06;
        } else if (selectedId) {
          edgeAlpha = isConnectedToSelected ? 0.55 : 0.06;
        } else if (hoveredId) {
          edgeAlpha = isConnectedToHovered ? 0.6 : 0.12;
        } else {
          edgeAlpha = 0.18;
        }

        const isHighlighted = isConnectedToSelected || isConnectedToHovered;

        // Bezier control point
        const mx = (a.x + b.x) / 2 + (b.y - a.y) * 0.1;
        const my = (a.y + b.y) / 2 - (b.x - a.x) * 0.1;

        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.quadraticCurveTo(mx, my, b.x, b.y);
        ctx.strokeStyle = isHighlighted
          ? `rgba(123,92,246,${edgeAlpha})`
          : `rgba(140,140,180,${edgeAlpha})`;
        ctx.lineWidth = (isHighlighted ? 1.2 : 0.5) / cam.zoom;
        ctx.stroke();

        // Particle along bezier
        edge.particleOffset = (edge.particleOffset + 0.0018) % 1;
        const tt = edge.particleOffset;
        const px = (1 - tt) * (1 - tt) * a.x + 2 * (1 - tt) * tt * mx + tt * tt * b.x;
        const py = (1 - tt) * (1 - tt) * a.y + 2 * (1 - tt) * tt * my + tt * tt * b.y;
        const pAlpha = isHighlighted ? 0.9 : 0.35;
        ctx.beginPath();
        ctx.arc(px, py, 1.5 / cam.zoom, 0, Math.PI * 2);
        ctx.fillStyle = isHighlighted
          ? `rgba(160,130,255,${pAlpha})`
          : `rgba(200,200,255,${pAlpha})`;
        ctx.fill();
      }

      // ── Draw nodes ─────────────────────────────────────────────────────────
      for (const n of simNodes) {
        const color = NODE_COLORS[n.node_type] ?? "#818CF8";
        const isHovered = n.id === hoveredId;
        const isSelected = n.id === selectedId;
        const isInFocusSet = !focusNeighbors || focusNeighbors.has(n.id);
        const isDimmed = focusNeighbors
          ? !isInFocusSet
          : selectedId
          ? !isSelected && !simEdges.some((e) => (e.from === n.id && e.to === selectedId) || (e.to === n.id && e.from === selectedId))
          : false;

        ctx.globalAlpha = isDimmed ? 0.18 : 1;

        // Pulse
        const phaseSeed = n.id.charCodeAt(0) * 0.8;
        const pulse = n.node_type === "self"
          ? 1
          : Math.sin(t * 0.001 + phaseSeed) * 0.12 + 1;
        const r = n.radius * (isHovered || isSelected ? 1.15 : 1) * pulse;

        // Outer glow
        if (isSelected || isHovered) {
          const grd = ctx.createRadialGradient(n.x, n.y, r * 0.5, n.x, n.y, r * 2.4);
          grd.addColorStop(0, color + "55");
          grd.addColorStop(1, color + "00");
          ctx.beginPath();
          ctx.arc(n.x, n.y, r * 2.4, 0, Math.PI * 2);
          ctx.fillStyle = grd;
          ctx.fill();
        }

        // Halo rings for self node
        if (n.node_type === "self") {
          ctx.beginPath();
          ctx.arc(n.x, n.y, r * 1.5, 0, Math.PI * 2);
          ctx.strokeStyle = color + "30";
          ctx.lineWidth = 1 / cam.zoom;
          ctx.stroke();
          ctx.beginPath();
          ctx.arc(n.x, n.y, r * 2.0, 0, Math.PI * 2);
          ctx.strokeStyle = color + "15";
          ctx.lineWidth = 0.5 / cam.zoom;
          ctx.stroke();
        }

        // Node body — radial gradient
        const bodyGrd = ctx.createRadialGradient(
          n.x - r * 0.3, n.y - r * 0.3, 0,
          n.x, n.y, r
        );
        bodyGrd.addColorStop(0, color + "ff");
        bodyGrd.addColorStop(0.5, color + "cc");
        bodyGrd.addColorStop(1, color + "66");
        ctx.beginPath();
        ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
        ctx.fillStyle = bodyGrd;
        ctx.fill();

        // Inner highlight
        ctx.beginPath();
        ctx.arc(n.x - r * 0.25, n.y - r * 0.25, r * 0.35, 0, Math.PI * 2);
        ctx.fillStyle = "rgba(255,255,255,0.18)";
        ctx.fill();

        // Selected ring
        if (isSelected) {
          ctx.beginPath();
          ctx.arc(n.x, n.y, r + 2 / cam.zoom, 0, Math.PI * 2);
          ctx.strokeStyle = "rgba(255,255,255,0.8)";
          ctx.lineWidth = 1.5 / cam.zoom;
          ctx.stroke();
        }

        // Label
        if (showLabels) {
          const labelAlpha = isDimmed ? 0.15 : (isHovered || isSelected ? 1 : 0.7);
          ctx.globalAlpha = labelAlpha;
          const fontSize = Math.max(9, (n.node_type === "self" ? 12 : 10) / cam.zoom);
          ctx.font = `600 ${fontSize}px 'Syne', system-ui, sans-serif`;
          ctx.textAlign = "center";
          ctx.fillStyle = "#e2e2f0";
          const label = n.label.length > 16 ? n.label.slice(0, 15) + "…" : n.label;
          ctx.fillText(label, n.x, n.y + r + 12 / cam.zoom);
        }

        ctx.globalAlpha = 1;
      }

      ctx.restore();
    };

    rafRef.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(rafRef.current);
  }, [hoveredId, selectedNode, focusId, showLabels]);

  // ─── Canvas sizing with DPR ─────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      starsRef.current = []; // regenerate stars on resize
      cameraRef.current.x = rect.width / 2;
      cameraRef.current.y = rect.height / 2;
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);
    return () => ro.disconnect();
  }, []);

  // ─── Hit test helper ────────────────────────────────────────────────────────
  const hitTest = useCallback((mx: number, my: number): SimNode | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const cam = cameraRef.current;
    const rect = canvas.getBoundingClientRect();
    const cx = (mx - rect.left - cam.x) / cam.zoom;
    const cy = (my - rect.top - cam.y) / cam.zoom;
    for (const n of [...simNodesRef.current].reverse()) {
      const dx = cx - n.x;
      const dy = cy - n.y;
      if (dx * dx + dy * dy <= n.radius * n.radius * 1.5) return n;
    }
    return null;
  }, []);

  // ─── Mouse events ──────────────────────────────────────────────────────────
  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (draggingNodeRef.current) {
        const cam = cameraRef.current;
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const cx = (e.clientX - rect.left - cam.x) / cam.zoom;
        const cy = (e.clientY - rect.top - cam.y) / cam.zoom;
        draggingNodeRef.current.x = cx;
        draggingNodeRef.current.y = cy;
        draggingNodeRef.current.vx = 0;
        draggingNodeRef.current.vy = 0;
        return;
      }
      if (isDraggingCanvas.current) {
        cameraRef.current.x = canvasDragStart.current.cx + (e.clientX - canvasDragStart.current.mx);
        cameraRef.current.y = canvasDragStart.current.cy + (e.clientY - canvasDragStart.current.my);
        return;
      }
      const hit = hitTest(e.clientX, e.clientY);
      setHoveredId(hit?.id ?? null);
    },
    [hitTest]
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      const hit = hitTest(e.clientX, e.clientY);
      if (hit) {
        draggingNodeRef.current = hit;
        hit.pinned = true;
        frozenRef.current = false;
      } else {
        isDraggingCanvas.current = true;
        canvasDragStart.current = {
          mx: e.clientX,
          my: e.clientY,
          cx: cameraRef.current.x,
          cy: cameraRef.current.y,
        };
      }
    },
    [hitTest]
  );

  const handleMouseUp = useCallback(
    (_e: React.MouseEvent) => {
      if (draggingNodeRef.current) {
        // Keep pinned — user dragged it to a position
      }
      draggingNodeRef.current = null;
      isDraggingCanvas.current = false;
    },
    []
  );

  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      const hit = hitTest(e.clientX, e.clientY);
      if (hit) {
        const fullNode = nodes.find((n) => n.id === hit.id) ?? null;
        setSelectedNode(fullNode);
      } else {
        setSelectedNode(null);
      }
    },
    [hitTest, nodes]
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      const hit = hitTest(e.clientX, e.clientY);
      if (hit) {
        setFocusId((prev) => (prev === hit.id ? null : hit.id));
      } else {
        setFocusId(null);
      }
    },
    [hitTest]
  );

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    cameraRef.current.zoom = Math.max(0.2, Math.min(4, cameraRef.current.zoom * factor));
  }, []);

  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") {
      setFocusId(null);
      setSelectedNode(null);
    }
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  // Parse node data field for meta/tags
  const parseNodeData = (n: GraphNode) => {
    let meta = `${n.mentions_count} mention${n.mentions_count !== 1 ? "s" : ""} · ${n.sentiment}`;
    let tags: string[] = [n.node_type];
    if (n.data) {
      try {
        const parsed = JSON.parse(n.data);
        if (parsed.meta) meta = parsed.meta;
        if (Array.isArray(parsed.tags)) tags = parsed.tags;
        if (parsed.description) meta = parsed.description;
      } catch {
        // raw string — show first 60 chars
        if (n.data.length <= 60) meta = n.data;
      }
    }
    return { meta, tags };
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "#09090f",
        fontFamily: "'Syne', system-ui, sans-serif",
        position: "relative",
        overflow: "hidden",
      }}
    >
      {/* Lens filter bar */}
      <GraphLensBar activeLens={lens} onChange={setLens} />

      {/* Canvas area */}
      <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
        <canvas
          ref={canvasRef}
          style={{
            width: "100%",
            height: "100%",
            display: "block",
            cursor: hoveredId ? "pointer" : isDraggingCanvas.current ? "grabbing" : "grab",
          }}
          onMouseMove={handleMouseMove}
          onMouseDown={handleMouseDown}
          onMouseUp={handleMouseUp}
          onClick={handleClick}
          onDoubleClick={handleDoubleClick}
          onWheel={handleWheel}
        />

        {/* ── Header overlay ───────────────────────────────────────── */}
        <div
          style={{
            position: "absolute",
            top: 14,
            left: 20,
            display: "flex",
            alignItems: "center",
            gap: 10,
            pointerEvents: "none",
          }}
        >
          <div
            style={{
              width: 28,
              height: 28,
              background: "linear-gradient(135deg, #7B5CF6, #38BDF8)",
              borderRadius: 8,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 13,
              fontWeight: 700,
              color: "#fff",
              flexShrink: 0,
            }}
          >
            N
          </div>
          <div>
            <div style={{ fontSize: 13, fontWeight: 600, color: "#e2e2f0", letterSpacing: "0.04em" }}>
              NodeMind · Knowledge Graph
            </div>
            <div
              style={{
                fontSize: 10,
                color: "#555570",
                fontFamily: "'DM Mono', monospace",
                marginTop: 2,
              }}
            >
              {nodes.length} nodes · {edges.length} connections
            </div>
          </div>
        </div>

        {/* ── Legend overlay ──────────────────────────────────────── */}
        <div
          style={{
            position: "absolute",
            top: 14,
            right: 56,
            display: "flex",
            gap: 10,
            alignItems: "center",
            pointerEvents: "none",
          }}
        >
          {[
            { color: "#7B5CF6", label: "self" },
            { color: "#38BDF8", label: "project" },
            { color: "#34D399", label: "goal" },
            { color: "#FBBF24", label: "person" },
            { color: "#F87171", label: "fog" },
            { color: "#818CF8", label: "idea" },
          ].map(({ color, label }) => (
            <div key={label} style={{ display: "flex", alignItems: "center", gap: 5 }}>
              <div
                style={{
                  width: 7, height: 7,
                  borderRadius: "50%",
                  background: color,
                  flexShrink: 0,
                }}
              />
              <span
                style={{
                  fontSize: 10,
                  color: "#555570",
                  fontFamily: "'DM Mono', monospace",
                }}
              >
                {label}
              </span>
            </div>
          ))}
        </div>

        {/* ── Toolbar (right) ──────────────────────────────────────── */}
        <div
          style={{
            position: "absolute",
            top: 50,
            right: 16,
            display: "flex",
            flexDirection: "column",
            gap: 6,
          }}
        >
          {[
            {
              title: "Reset view",
              icon: "⟲",
              onClick: () => {
                const canvas = canvasRef.current;
                if (canvas) {
                  const rect = canvas.getBoundingClientRect();
                  cameraRef.current = { x: rect.width / 2, y: rect.height / 2, zoom: 1 };
                }
              },
            },
            {
              title: "Toggle labels",
              icon: "T",
              onClick: () => setShowLabels((v) => !v),
            },
          ].map(({ title, icon, onClick }) => (
            <button
              key={title}
              title={title}
              onClick={onClick}
              style={{
                width: 28,
                height: 28,
                background: "rgba(255,255,255,0.05)",
                border: "0.5px solid rgba(255,255,255,0.08)",
                borderRadius: 8,
                color: "#555570",
                fontSize: 13,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                cursor: "pointer",
                transition: "background 0.15s, color 0.15s",
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLButtonElement).style.background = "rgba(123,92,246,0.15)";
                (e.currentTarget as HTMLButtonElement).style.color = "#9d86f5";
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.05)";
                (e.currentTarget as HTMLButtonElement).style.color = "#555570";
              }}
            >
              {icon}
            </button>
          ))}
        </div>

        {/* ── Bottom bar: node info + stats ─────────────────────────── */}
        <div
          style={{
            position: "absolute",
            bottom: 14,
            left: 16,
            right: 16,
            display: "flex",
            alignItems: "flex-end",
            justifyContent: "space-between",
            pointerEvents: "none",
          }}
        >
          {/* Node info card */}
          <div
            style={{
              background: "rgba(255,255,255,0.04)",
              border: "0.5px solid rgba(255,255,255,0.08)",
              borderRadius: 10,
              padding: "10px 14px",
              maxWidth: 280,
              opacity: selectedNode ? 1 : 0,
              transition: "opacity 0.25s",
            }}
          >
            {selectedNode && (() => {
              const { meta, tags } = parseNodeData(selectedNode);
              return (
                <>
                  <div style={{ fontSize: 13, fontWeight: 600, color: "#e2e2f0", marginBottom: 2 }}>
                    {selectedNode.label}
                  </div>
                  <div
                    style={{
                      fontSize: 10,
                      fontFamily: "'DM Mono', monospace",
                      color: "#6666aa",
                      marginBottom: 6,
                    }}
                  >
                    {selectedNode.node_type.toUpperCase().replace("_", " ")} · {meta}
                  </div>
                  <div>
                    {tags.map((tag) => (
                      <span
                        key={tag}
                        style={{
                          display: "inline-block",
                          background: "rgba(123,92,246,0.15)",
                          border: "0.5px solid rgba(123,92,246,0.3)",
                          borderRadius: 4,
                          padding: "1px 6px",
                          fontSize: 9,
                          fontFamily: "'DM Mono', monospace",
                          color: "#9d86f5",
                          marginRight: 4,
                          marginTop: 4,
                        }}
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                </>
              );
            })()}
          </div>

          {/* Stats pills */}
          <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-end" }}>
            {[
              { label: "nodes", value: nodes.length },
              { label: "edges", value: edges.length },
              {
                label: "momentum",
                value: (() => {
                  const vals = Object.values(momentumScores);
                  if (!vals.length) return "—";
                  const avg = vals.reduce((a, b) => a + b, 0) / vals.length;
                  return `${Math.round(avg * 100)}%`;
                })(),
              },
            ].map(({ label, value }) => (
              <div
                key={label}
                style={{
                  background: "rgba(255,255,255,0.04)",
                  border: "0.5px solid rgba(255,255,255,0.07)",
                  borderRadius: 20,
                  padding: "5px 12px",
                  fontSize: 10,
                  fontFamily: "'DM Mono', monospace",
                  color: "#555570",
                  display: "flex",
                  gap: 6,
                  alignItems: "center",
                }}
              >
                {label}{" "}
                <span style={{ color: "#9d9dcc", fontWeight: 500 }}>{value}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Focus mode hint */}
        {focusId && (
          <div
            style={{
              position: "absolute",
              top: "50%",
              left: "50%",
              transform: "translate(-50%, -180px)",
              background: "rgba(123,92,246,0.15)",
              border: "0.5px solid rgba(123,92,246,0.35)",
              borderRadius: 20,
              padding: "4px 14px",
              fontSize: 10,
              fontFamily: "'DM Mono', monospace",
              color: "#9d86f5",
              pointerEvents: "none",
            }}
          >
            1-hop focus · double-click or Esc to exit
          </div>
        )}

        {/* Empty state */}
        {nodes.length === 0 && (
          <div
            style={{
              position: "absolute",
              top: "50%",
              left: "50%",
              transform: "translate(-50%,-50%)",
              textAlign: "center",
              color: "rgba(255,255,255,0.35)",
              pointerEvents: "none",
            }}
          >
            <div style={{ fontSize: 32, marginBottom: 8 }}>◎</div>
            <div style={{ fontSize: 14 }}>
              No nodes yet. Start a voice session to build your knowledge graph.
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default KnowledgeGraphView;
