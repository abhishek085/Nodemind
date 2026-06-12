import React, { useState, useRef, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface TooltipProps {
  label: string;
  description: string;
  /** Optional context string sent to Gemma for a richer insight */
  aiContext?: string;
  children: React.ReactNode;
  position?: "top" | "bottom" | "left" | "right";
}

/**
 * Tooltip with static description + optional AI-powered insight via Gemma.
 * Hover for 400ms → shows immediately with static text.
 * Click the ✦ button → loads a Gemma-generated 1-2 sentence explanation.
 */
const Tooltip: React.FC<TooltipProps> = ({
  label,
  description,
  aiContext,
  children,
  position = "bottom",
}) => {
  const [visible, setVisible] = useState(false);
  const [aiInsight, setAiInsight] = useState<string | null>(null);
  const [aiLoading, setAiLoading] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

  const showTooltip = useCallback(() => {
    timerRef.current = setTimeout(() => setVisible(true), 380);
  }, []);

  const hideTooltip = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setVisible(false);
    setAiInsight(null);
    setAiLoading(false);
  }, []);

  const loadAiInsight = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (aiLoading || aiInsight) return;
    setAiLoading(true);
    try {
      const ctx = aiContext ?? description;
      const insight = await invoke<string>("ask_gemma_tooltip", {
        concept: label,
        context: ctx,
      });
      setAiInsight(insight);
    } catch {
      setAiInsight("AI insight unavailable — start Ollama to enable.");
    } finally {
      setAiLoading(false);
    }
  };

  // Close on outside click
  useEffect(() => {
    if (!visible) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        hideTooltip();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [visible, hideTooltip]);

  return (
    <div
      ref={wrapRef}
      className="tooltip-wrap"
      onMouseEnter={showTooltip}
      onMouseLeave={hideTooltip}
    >
      {children}
      {visible && (
        <div className={`tooltip-bubble tooltip-${position}`}>
          <div className="tooltip-title">{label}</div>
          <div className="tooltip-desc">{description}</div>
          {aiContext !== undefined && (
            <div className="tooltip-ai-row">
              {aiLoading ? (
                <span className="tooltip-ai-loading">Gemma thinking…</span>
              ) : aiInsight ? (
                <span className="tooltip-ai-text">{aiInsight}</span>
              ) : (
                <button className="tooltip-ai-btn" onClick={loadAiInsight}>
                  ✦ Ask Gemma
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default Tooltip;
