// Mirrors the backend capture session (audio::session) for one window.
//
// Capture can start anywhere — this window, the menu-bar popover, or the
// global hotkey — so state comes from `capture-*` events, never from which
// button was clicked here.

import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import type { CaptureMode, CaptureResult, CaptureStage } from "./types";

export type CapturePhase =
  | { name: "idle" }
  | { name: "recording" }
  | { name: "processing"; stage: CaptureStage }
  | { name: "result"; result: CaptureResult };

export interface CaptureController {
  phase: CapturePhase;
  /** Mode of the current (or last) capture. */
  mode: CaptureMode;
  seconds: number;
  /** Mic level (You). */
  level: number;
  /** Call-audio level (Them), meeting mode only. */
  themLevel: number;
  /** Live transcript so far (filled in at each pause while recording). */
  partial: string;
  /** No input at all after a few seconds — wrong device or no permission. */
  micDead: boolean;
  error: string | null;
  /** Non-fatal, e.g. "recording your mic only". */
  warning: string | null;
  start: (language?: string, mode?: CaptureMode) => Promise<void>;
  stop: () => Promise<void>;
  cancel: () => Promise<void>;
  /** Leave the result screen. */
  reset: () => void;
  /** Update the shown result (e.g. after undoing a task). */
  setResult: (r: CaptureResult) => void;
}

export function useCapture(): CaptureController {
  const [phase, setPhase] = useState<CapturePhase>({ name: "idle" });
  const [seconds, setSeconds] = useState(0);
  const [mode, setMode] = useState<CaptureMode>("note");
  const [level, setLevel] = useState(0);
  const [themLevel, setThemLevel] = useState(0);
  const [warning, setWarning] = useState<string | null>(null);
  const [peak, setPeak] = useState(0);
  const [partial, setPartial] = useState("");
  const [error, setError] = useState<string | null>(null);
  const startedAt = useRef<number | null>(null);

  const beginRecording = useCallback((elapsedSecs: number, text: string, m: CaptureMode) => {
    startedAt.current = Date.now() - elapsedSecs * 1000;
    setMode(m);
    setSeconds(elapsedSecs);
    setLevel(0);
    setThemLevel(0);
    setWarning(null);
    setPeak(0);
    setPartial(text);
    setError(null);
    setPhase({ name: "recording" });
  }, []);

  // A window opened mid-capture picks up where the session is.
  useEffect(() => {
    void api
      .captureStatus()
      .then((s) => s.recording && beginRecording(s.elapsedSecs, s.partial, s.mode ?? "note"))
      .catch(() => undefined);
  }, [beginRecording]);

  useEffect(() => {
    if (phase.name !== "recording") return;
    const t = window.setInterval(() => {
      if (startedAt.current !== null) {
        setSeconds(Math.floor((Date.now() - startedAt.current) / 1000));
      }
    }, 500);
    return () => window.clearInterval(t);
  }, [phase.name]);

  useEffect(() => {
    const subs = [
      api.on<{ state: "recording" | "processing" | "idle"; mode?: CaptureMode }>("capture-state", (e) => {
        if (e.state === "recording") beginRecording(0, "", e.mode ?? "note");
        else if (e.state === "processing") setPhase({ name: "processing", stage: "transcribing" });
        // "idle" after processing is followed by capture-complete/-error;
        // on its own it means the capture was cancelled.
        else setPhase((p) => (p.name === "recording" ? { name: "idle" } : p));
      }),
      api.on<{ stage: CaptureStage }>("capture-stage", (e) =>
        setPhase((p) =>
          p.name === "processing" && e.stage !== "done" ? { name: "processing", stage: e.stage } : p,
        ),
      ),
      api.on<{ level: number; source: "mic" | "system" }>("capture-level", (e) => {
        if (e.source === "system") {
          setThemLevel(e.level);
          return;
        }
        setLevel(e.level);
        setPeak((p) => Math.max(p, e.level));
      }),
      api.on<{ message: string }>("capture-warning", (e) => setWarning(e.message)),
      api.on<{ text: string }>("capture-partial", (e) => setPartial(e.text)),
      api.on<CaptureResult>("capture-complete", (result) => setPhase({ name: "result", result })),
      api.on<{ message: string }>("capture-error", (e) => {
        setError(e.message);
        setPhase({ name: "idle" });
      }),
    ];
    return () => {
      for (const un of subs) void un.then((f) => f());
    };
  }, [beginRecording]);

  const start = useCallback(async (language?: string, m: CaptureMode = "note") => {
    setError(null);
    try {
      await api.startCapture(language, m);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const stop = useCallback(async () => {
    try {
      await api.stopCapture(); // the result arrives as `capture-complete`
    } catch (e) {
      setError(String(e));
      setPhase({ name: "idle" });
    }
  }, []);

  const cancel = useCallback(async () => {
    await api.cancelCapture().catch(() => undefined);
    setPhase({ name: "idle" });
  }, []);

  const reset = useCallback(() => setPhase({ name: "idle" }), []);
  const setResult = useCallback((result: CaptureResult) => setPhase({ name: "result", result }), []);

  return {
    phase,
    mode,
    seconds,
    level,
    themLevel,
    partial,
    micDead: phase.name === "recording" && seconds >= 3 && peak < 0.02,
    error,
    warning,
    start,
    stop,
    cancel,
    reset,
    setResult,
  };
}
