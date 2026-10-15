// Capture UI pieces shared by the full Capture view and the menu-bar popover.
// Purely presentational: state lives in lib/useCapture.ts.

import { useState } from "react";
import * as api from "../lib/api";
import { FogCard } from "./fog";
import { ItemGroups } from "./items";
import type { ActionItem, CaptureMode, CaptureResult, CaptureStage } from "../lib/types";

export function fmtClock(s: number): string {
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

export function MicButton({ onClick, size = "lg" }: { onClick: () => void; size?: "lg" | "md" }) {
  const dim = size === "lg" ? "h-28 w-28" : "h-20 w-20";
  return (
    <button
      onClick={onClick}
      className={`flex ${dim} items-center justify-center rounded-full bg-gradient-to-br from-accent to-accent-2 text-white shadow-md transition-transform hover:scale-105 active:scale-95`}
      title="Start recording (⌥⇧Space)"
    >
      <svg viewBox="0 0 24 24" className={size === "lg" ? "h-10 w-10" : "h-8 w-8"} fill="none" stroke="currentColor" strokeWidth="1.8">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0M12 18v3" strokeLinecap="round" />
      </svg>
    </button>
  );
}

export function StopButton({ onClick, size = "lg" }: { onClick: () => void; size?: "lg" | "md" }) {
  const dim = size === "lg" ? "h-28 w-28" : "h-20 w-20";
  return (
    <button
      onClick={onClick}
      className={`rec-pulse flex ${dim} items-center justify-center rounded-full bg-danger text-white shadow-md transition-transform hover:scale-105 active:scale-95`}
      title="Stop and process"
    >
      <span className={`${size === "lg" ? "h-9 w-9" : "h-7 w-7"} rounded-md bg-white`} />
    </button>
  );
}

/** Live input level, so it's obvious the mic is actually hearing you. */
export function LevelMeter({ level }: { level: number }) {
  return (
    <div className="mt-4 h-1.5 w-40 overflow-hidden rounded-full bg-surface-2" aria-hidden>
      <div
        className="h-full rounded-full bg-accent transition-[width] duration-150 ease-out"
        style={{ width: `${Math.round(level * 100)}%` }}
      />
    </div>
  );
}

/** Brain-dump or meeting notes. */
export function ModeSwitch({ mode, onMode }: { mode: CaptureMode; onMode: (m: CaptureMode) => void }) {
  return (
    <div className="flex gap-1 rounded-full bg-surface-2 p-1">
      {(
        [
          ["note", "Brain-dump"],
          ["meeting", "Meeting"],
        ] as const
      ).map(([m, label]) => (
        <button
          key={m}
          onClick={() => onMode(m)}
          className={`rounded-full px-3 py-1 text-xs font-semibold transition-colors ${
            mode === m ? "bg-surface text-ink shadow-sm" : "text-ink-3 hover:text-ink-2"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

/** Meeting mode: one meter per side, so a silent side is obvious. */
export function DualMeter({ you, them }: { you: number; them: number }) {
  const bar = (label: string, level: number) => (
    <div className="flex items-center gap-2">
      <span className="w-9 text-right text-[10px] font-semibold uppercase tracking-wide text-ink-3">{label}</span>
      <div className="h-1.5 w-32 overflow-hidden rounded-full bg-surface-2">
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-150 ease-out"
          style={{ width: `${Math.round(level * 100)}%` }}
        />
      </div>
    </div>
  );
  return (
    <div className="mt-4 space-y-1.5" aria-hidden>
      {bar("You", you)}
      {bar("Them", them)}
    </div>
  );
}

/** The transcript as it arrives — one piece per pause; meetings get speaker labels. */
export function LivePartial({ text, className = "" }: { text: string; className?: string }) {
  if (!text) return null;
  const lines = text.split("\n");
  const labelled = lines.every((l) => l.startsWith("You: ") || l.startsWith("Them: "));
  return (
    <div className={`max-h-40 overflow-y-auto text-left text-[13px] leading-relaxed text-ink-2 ${className}`}>
      {labelled
        ? lines.map((l, i) => {
            const you = l.startsWith("You: ");
            return (
              <p key={i} className="mb-1">
                <span className={`font-semibold ${you ? "text-accent" : "text-ink"}`}>{you ? "You" : "Them"}:</span>{" "}
                {l.slice(you ? 5 : 6)}
              </p>
            );
          })
        : <p className="whitespace-pre-wrap">{text}</p>}
    </div>
  );
}

export function RecordingHint({ micDead }: { micDead: boolean }) {
  return (
    <p className={`mt-2 text-xs ${micDead ? "text-danger" : "text-ink-3"}`}>
      {micDead
        ? "Not hearing anything — check System Settings → Privacy & Security → Microphone, and the input under Sound"
        : "Tap the square (or ⌥⇧Space) to stop"}
    </p>
  );
}

const STAGE_LABEL: Record<CaptureStage, string> = {
  transcribing: "Finishing the transcript…",
  organizing: "Organizing into checklist and mind map…",
  done: "Done",
};

export function Processing({ stage }: { stage: CaptureStage }) {
  return (
    <div className="flex flex-col items-center">
      <div className="h-10 w-10 animate-spin rounded-full border-2 border-line border-t-accent" />
      <p className="mt-5 text-sm text-ink-2">{STAGE_LABEL[stage]}</p>
    </div>
  );
}

export function NothingHeard({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="flex flex-col items-center text-center">
      <h3 className="text-sm font-bold">Didn't catch any speech</h3>
      <p className="mt-2 max-w-sm text-xs text-ink-2">
        The recording was silent or too short. Watch the level bar while you talk — if it stays
        flat, pick the right microphone in System Settings → Sound → Input.
      </p>
      <button
        onClick={onRetry}
        className="mt-5 rounded-xl bg-accent px-5 py-2 text-sm font-semibold text-white hover:opacity-90"
      >
        Try again
      </button>
    </div>
  );
}

/**
 * What a capture produced. Tasks are already on Today — each can be undone
 * here, which is quicker than accepting items one by one. Meetings show
 * minutes instead of a brain-dump checklist.
 */
export function ResultBody({
  result,
  onChange,
  compact = false,
}: {
  result: CaptureResult;
  onChange: (r: CaptureResult) => void;
  compact?: boolean;
}) {
  if (result.noise) return <NothingToKeep result={result} />;
  if (result.minutes) return <MinutesBody result={result} onChange={onChange} compact={compact} />;
  return <NoteBody result={result} onChange={onChange} compact={compact} />;
}

interface BodyProps {
  result: CaptureResult;
  onChange: (r: CaptureResult) => void;
  compact: boolean;
}

/** Tasks added to Today, each undoable. Shared by notes and meetings. */
function AddedTasks({ result, onChange, heading }: Omit<BodyProps, "compact"> & { heading: string }) {
  const [busy, setBusy] = useState<string | null>(null);
  const undo = async (id: string) => {
    setBusy(id);
    try {
      await api.deleteTask(id);
      onChange({ ...result, tasks: result.tasks.filter((t) => t.id !== id) });
    } finally {
      setBusy(null);
    }
  };
  return (
    <div>
      <h3 className="mb-2 text-sm font-bold">
        {heading} <span className="font-normal text-ink-3">— undo anything that's wrong</span>
      </h3>
      <ul className="space-y-1.5">
        {result.tasks.map((t) => (
          <li key={t.id} className="flex items-start gap-2.5">
            <span className="mt-0.5 text-ok">✓</span>
            <span className="flex-1 text-[13px] text-ink">{t.title}</span>
            <button
              onClick={() => void undo(t.id)}
              disabled={busy === t.id}
              className="rounded-md px-2 py-0.5 text-[11px] text-ink-3 hover:bg-surface-2 hover:text-danger"
            >
              Undo
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Compact "Show transcript" for the popover result. */
export function TranscriptToggle({ text }: { text: string }) {
  return (
    <details className="mt-3 text-left text-xs text-ink-3">
      <summary className="cursor-pointer">Show transcript</summary>
      <LivePartial text={text} className="mt-2 max-h-48" />
    </details>
  );
}

function TranscriptDetails({ result }: { result: CaptureResult }) {
  return (
    <details className="text-xs text-ink-3">
      <summary className="cursor-pointer">
        Transcript · {Math.max(1, Math.round(result.durationSecs / 60))} min ·{" "}
        {result.transcript.split(/\s+/).length} words
      </summary>
      <LivePartial text={result.transcript} className="mt-2 max-h-56" />
    </details>
  );
}

/** Meeting minutes: summary, decisions, your action items (on Today), theirs. */
function MinutesBody({ result, onChange, compact }: BodyProps) {
  const m = result.minutes!;
  const mine = (a: ActionItem) => ["you", "me", "i", ""].includes(a.owner.trim().toLowerCase());
  const theirs = m.actionItems.filter((a) => !mine(a));
  return (
    <div className="space-y-4 text-left">
      <div>
        <h3 className="text-base font-bold">{m.title || "Meeting"}</h3>
        {m.summary && <p className="mt-1 text-[13px] leading-relaxed text-ink-2">{m.summary}</p>}
      </div>
      {m.decisions.length > 0 && (
        <div>
          <h4 className="mb-1.5 text-sm font-bold">Decisions</h4>
          <ul className="list-disc space-y-1 pl-5 text-[13px] text-ink">
            {m.decisions.map((d, i) => (
              <li key={i}>{d}</li>
            ))}
          </ul>
        </div>
      )}
      <FogCard signals={result.fog ?? []} />
      {result.tasks.length > 0 ? (
        <AddedTasks result={result} onChange={onChange} heading="Your action items, added to Today" />
      ) : (
        <p className="text-xs text-ink-3">No action items for you in this one.</p>
      )}
      {result.items.some((i) => i.kind !== "decision") && (
        <ItemGroups items={result.items.filter((i) => i.kind !== "decision")} />
      )}
      {theirs.length > 0 && (
        <div>
          <h4 className="mb-1.5 text-sm font-bold">Others' action items</h4>
          <ul className="space-y-1 text-[13px] text-ink-2">
            {theirs.map((a, i) => (
              <li key={i}>
                <span className="font-semibold text-ink">{a.owner}:</span> {a.task}
                {a.due && <span className="text-ink-3"> · {a.due}</span>}
              </li>
            ))}
          </ul>
        </div>
      )}
      {!compact && <TranscriptDetails result={result} />}
    </div>
  );
}

/** Brain-dump result: summary, tasks on Today, what joined the mind map. */
/** Small talk or a test: nothing was kept, and that's fine. */
function NothingToKeep({ result }: { result: CaptureResult }) {
  return (
    <div className="space-y-3 text-left">
      <p className="text-sm font-semibold">Nothing to keep</p>
      <p className="text-xs text-ink-3">
        That sounded like small talk or a test, so nothing went to your checklist or mind map.
        Say what you need to do, decided, wonder or realized, and it'll be picked up.
      </p>
      <TranscriptToggle text={result.transcript} />
    </div>
  );
}

/** Brain-dump: tasks (on Today), then what else was worth keeping. */
function NoteBody({ result, onChange, compact }: BodyProps) {
  const about = [...result.topics, ...result.people];
  return (
    <div className="space-y-4 text-left">
      {result.tasks.length > 0 && <AddedTasks result={result} onChange={onChange} heading="Added to Today" />}
      {result.items.length > 0 && <ItemGroups items={result.items} />}
      <FogCard signals={result.fog ?? []} />
      {about.length > 0 && (
        <p className="text-[11px] text-ink-3">
          About: {about.slice(0, compact ? 4 : 10).join(" · ")}
        </p>
      )}
      {!compact && <TranscriptDetails result={result} />}
    </div>
  );
}
