import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { GraphNode, GraphEdge, GraphData, DriftAlert, Task } from "../types";

interface MomentumScores {
  [nodeId: string]: number;
}

export function useGraphData() {
  const [graphData, setGraphData] = useState<GraphData>({ nodes: [], edges: [] });
  const [driftAlerts, setDriftAlerts] = useState<DriftAlert[]>([]);
  const [momentumScores, setMomentumScores] = useState<MomentumScores>({});
  const [recentTasks, setRecentTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchAll = useCallback(async () => {
    try {
      const [nodes, edges, alerts, scores, tasks] = await Promise.all([
        invoke<GraphNode[]>("get_graph_nodes"),
        invoke<GraphEdge[]>("get_graph_edges"),
        invoke<DriftAlert[]>("get_drift_alerts"),
        invoke<MomentumScores>("get_momentum_scores"),
        invoke<Task[]>("get_tasks"),
      ]);
      setGraphData({ nodes, edges });
      setDriftAlerts(alerts);
      setMomentumScores(scores);
      setRecentTasks(tasks);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchAll();
    // Re-fetch whenever the backend emits a graph update event
    const unlisten = listen("mental-map-updated", () => {
      fetchAll();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [fetchAll]);

  const cutoff24h = Date.now() - 86400000;
  const cutoff48h = Date.now() - 2 * 86400000;

  // Nodes updated in the last 24h — passive extraction feed for dashboard
  const recentExtractions = [...graphData.nodes]
    .filter((n) => new Date(n.last_mentioned_at ?? n.created_at).getTime() > cutoff24h)
    .sort((a, b) => {
      const ta = new Date(a.last_mentioned_at ?? a.created_at).getTime();
      const tb = new Date(b.last_mentioned_at ?? b.created_at).getTime();
      return tb - ta;
    })
    .slice(0, 12);

  // Fog-type nodes from the last 24h
  const fogSignals = graphData.nodes.filter(
    (n) =>
      (n.node_type === "fog_pattern" || n.node_type === "fog") &&
      new Date(n.last_mentioned_at ?? n.created_at).getTime() > cutoff24h
  );

  // Person nodes mentioned in the last 48h
  const recentPeople = graphData.nodes
    .filter(
      (n) =>
        n.node_type === "person" &&
        new Date(n.last_mentioned_at ?? n.created_at).getTime() > cutoff48h
    )
    .sort((a, b) => {
      const ta = new Date(a.last_mentioned_at ?? a.created_at).getTime();
      const tb = new Date(b.last_mentioned_at ?? b.created_at).getTime();
      return tb - ta;
    })
    .slice(0, 6);

  return {
    graphData,
    driftAlerts,
    momentumScores,
    recentTasks,
    recentExtractions,
    fogSignals,
    recentPeople,
    loading,
    error,
    refresh: fetchAll,
  };
}
