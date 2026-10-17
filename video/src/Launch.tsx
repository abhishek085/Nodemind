// Nodemind launch film, ~25 s, captions-first (feeds autoplay muted).
// Scenes: title → you ramble → it sorts → the map connects → search → local.
// Colours and type follow the product (see src/index.css in the app).

import { loadFont as loadDisplay } from "@remotion/google-fonts/BricolageGrotesque";
import { loadFont as loadBody } from "@remotion/google-fonts/IBMPlexSans";
import { loadFont as loadMono } from "@remotion/google-fonts/IBMPlexMono";
import { AbsoluteFill, Easing, Sequence, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";

export const FPS = 30;
const S = (sec: number) => Math.round(sec * FPS);
export const DURATION = S(26);

const display = loadDisplay().fontFamily;
const body = loadBody().fontFamily;
const mono = loadMono().fontFamily;

const C = {
  bg: "#0D1424", panel: "#131C30", line: "#24304A", ink: "#E7ECF5", muted: "#98A3B8",
  blue: "#5B8DFF", teal: "#2ED3C3", amber: "#F2B34B", violet: "#9B7FF0", red: "#FF6B66", green: "#4ADE80",
};
const KIND = {
  task: { label: "Task", color: C.green, mark: "✓" },
  decision: { label: "Decision", color: C.violet, mark: "●" },
  question: { label: "Question", color: C.blue, mark: "?" },
  idea: { label: "Idea", color: C.amber, mark: "✦" },
  learning: { label: "Learning", color: C.teal, mark: "◆" },
} as const;

const ease = Easing.bezier(0.16, 1, 0.3, 1);
const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;
const fade = (f: number, from: number, to: number) => interpolate(f, [from, to], [0, 1], { ...clamp, easing: ease });

const Caption = ({ children, sub, at = 0 }: { children: string; sub?: string; at?: number }) => {
  const f = useCurrentFrame();
  const o = fade(f, at, at + 14);
  return (
    <div style={{ position: "absolute", left: 120, right: 120, top: 70, opacity: o, transform: `translateY(${(1 - o) * 18}px)` }}>
      <div style={{ fontFamily: display, fontWeight: 800, fontSize: 76, letterSpacing: -2, color: C.ink, lineHeight: 1.05 }}>{children}</div>
      {sub && <div style={{ fontFamily: body, fontSize: 32, color: C.muted, marginTop: 14 }}>{sub}</div>}
    </div>
  );
};

const Chip = ({ kind, text, style }: { kind: keyof typeof KIND; text: string; style?: React.CSSProperties }) => {
  const k = KIND[kind];
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 14, background: C.panel, border: `2px solid ${C.line}`, borderLeft: `8px solid ${k.color}`, borderRadius: 18, padding: "16px 22px", fontFamily: body, fontSize: 30, color: C.ink, ...style }}>
      <span style={{ color: k.color, fontFamily: mono, fontSize: 24, fontWeight: 500, minWidth: 200 }}>{k.mark} {k.label}</span>
      <span>{text}</span>
    </div>
  );
};

// ── scenes ──────────────────────────────────────────────────────────────────

const Title = () => {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame: f, fps, config: { damping: 14 } });
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", gap: 28 }}>
      <svg width={150} height={150} viewBox="0 0 64 64" style={{ transform: `scale(${pop})` }}>
        <rect width="64" height="64" rx="14" fill={C.blue} />
        <path d="M32 32 18 20M32 32h16M32 32 18 44" stroke="#fff" strokeWidth="5" strokeLinecap="round" />
        <circle cx="32" cy="32" r="7" fill="#fff" />
        <circle cx="48" cy="32" r="5" fill="#00C4B4" />
      </svg>
      <div style={{ fontFamily: display, fontWeight: 800, fontSize: 150, letterSpacing: -5, color: C.ink, opacity: fade(f, 8, 24) }}>
        node<span style={{ color: C.teal }}>mind</span>
      </div>
      <div style={{ fontFamily: body, fontSize: 46, color: C.muted, opacity: fade(f, 24, 44) }}>Talk it out. Get a checklist.</div>
    </AbsoluteFill>
  );
};

const RAMBLE = [
  "Okay so, I need to call Priya about the invoice tomorrow.",
  "We decided to price it at nine dollars a month.",
  "I still don't know whether to offer a free tier.",
  "Maybe we could build an iPhone version.",
  "Hello, testing, one two three, okay cool…",
];

const Ramble = () => {
  const f = useCurrentFrame();
  const pressed = f > 10 && f < 80;
  return (
    <AbsoluteFill>
      <Caption sub="Hold a hotkey from any app and say what's on your mind.">You ramble.</Caption>
      <div style={{ position: "absolute", left: 120, top: 330, display: "flex", gap: 14, alignItems: "center", opacity: fade(f, 6, 20) }}>
        {["⌥", "⇧", "Space"].map((k) => (
          <div key={k} style={{ fontFamily: mono, fontSize: 34, color: C.ink, padding: "14px 26px", borderRadius: 14, background: pressed ? C.blue : C.panel, border: `2px solid ${pressed ? C.blue : C.line}`, transform: `translateY(${pressed ? 4 : 0}px)` }}>{k}</div>
        ))}
        <div style={{ fontFamily: body, fontSize: 28, color: pressed ? C.red : C.muted, marginLeft: 18 }}>{pressed ? "● listening" : ""}</div>
      </div>
      <div style={{ position: "absolute", left: 120, right: 200, top: 470, display: "flex", flexDirection: "column", gap: 26 }}>
        {RAMBLE.map((line, i) => {
          const start = 22 + i * 26;
          const shown = Math.floor(interpolate(f, [start, start + 24], [0, line.length], clamp));
          return (
            <div key={line} style={{ fontFamily: body, fontSize: 40, color: i === 4 ? C.muted : C.ink, minHeight: 52 }}>
              {line.slice(0, shown)}
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};

const SORTED: { kind: keyof typeof KIND; text: string }[] = [
  { kind: "task", text: "Call Priya about the invoice tomorrow" },
  { kind: "decision", text: "We decided to price it at $9 a month" },
  { kind: "question", text: "Should I offer a free tier?" },
  { kind: "idea", text: "Build an iPhone version" },
];

const Sort = () => {
  const f = useCurrentFrame();
  return (
    <AbsoluteFill>
      <Caption sub="Tasks, ideas, questions, decisions — and small talk gets dropped.">It sorts what you said.</Caption>
      <div style={{ position: "absolute", left: 120, right: 120, top: 340, display: "flex", flexDirection: "column", gap: 22 }}>
        {SORTED.map((s, i) => {
          const o = fade(f, 14 + i * 22, 36 + i * 22);
          return <Chip key={s.text} {...s} style={{ opacity: o, transform: `translateX(${(1 - o) * 80}px)` }} />;
        })}
        <div style={{ display: "flex", alignItems: "center", gap: 18, opacity: fade(f, 110, 130), fontFamily: body, fontSize: 30, color: C.muted, padding: "10px 24px" }}>
          <span style={{ textDecoration: "line-through" }}>“Hello, testing, one two three…”</span>
          <span style={{ fontFamily: mono, fontSize: 22, color: C.red }}>nothing to keep</span>
        </div>
      </div>
      <div style={{ position: "absolute", right: 120, bottom: 70, fontFamily: body, fontSize: 28, color: C.green, opacity: fade(f, 90, 110) }}>
        ✓ Tasks go straight to Today. Duplicates are skipped.
      </div>
    </AbsoluteFill>
  );
};

const NODES: { id: string; x: number; y: number; kind?: keyof typeof KIND; label: string; topic?: boolean }[] = [
  { id: "pricing", x: 960, y: 380, label: "pricing", topic: true },
  { id: "d1", x: 640, y: 300, kind: "decision", label: "Price at $9 a month" },
  { id: "q1", x: 1010, y: 640, kind: "question", label: "Offer a free tier?" },
  { id: "i1", x: 1300, y: 330, kind: "idea", label: "iPhone version" },
  { id: "app", x: 1480, y: 600, label: "app", topic: true },
  { id: "t1", x: 520, y: 600, kind: "task", label: "Call Priya re invoice" },
  { id: "priya", x: 330, y: 400, label: "Priya", topic: true },
];
const EDGES: [string, string, "about" | "similar"][] = [
  ["d1", "pricing", "about"], ["q1", "pricing", "about"], ["i1", "app", "about"], ["t1", "priya", "about"], ["d1", "q1", "similar"], ["i1", "q1", "similar"],
];

const MapScene = () => {
  const f = useCurrentFrame();
  const at = (id: string) => NODES.find((n) => n.id === id)!;
  return (
    <AbsoluteFill>
      <Caption sub="Ideas connect to topics and to similar ideas from other days.">It all connects.</Caption>
      <svg width={1920} height={1080} style={{ position: "absolute", inset: 0 }}>
        {EDGES.map(([a, b, kind], i) => {
          const o = fade(f, 40 + i * 8, 60 + i * 8);
          const p = at(a), q = at(b);
          return <line key={a + b} x1={p.x} y1={p.y + 110} x2={q.x} y2={q.y + 110} stroke={kind === "similar" ? C.blue : "#3A4766"} strokeWidth={kind === "similar" ? 4 : 3} strokeDasharray={kind === "similar" ? "14 10" : undefined} opacity={o} />;
        })}
      </svg>
      {NODES.map((n, i) => {
        const o = fade(f, 8 + i * 6, 26 + i * 6);
        const k = n.kind && KIND[n.kind];
        return (
          <div key={n.id} style={{ position: "absolute", left: n.x, top: n.y + 110, transform: `translate(-50%,-50%) scale(${0.8 + 0.2 * o})`, opacity: o }}>
            {n.topic ? (
              <div style={{ fontFamily: body, fontWeight: 600, fontSize: 32, color: C.violet, padding: "10px 26px", borderRadius: 999, border: `2px solid ${C.violet}88`, background: "#1F2146" }}>{n.label}</div>
            ) : (
              <div style={{ fontFamily: body, fontSize: 26, color: C.ink, padding: "14px 22px", borderRadius: 16, background: C.panel, border: `2px solid ${C.line}`, borderLeft: `7px solid ${k!.color}`, width: 300 }}>
                <div style={{ fontFamily: mono, fontSize: 18, color: k!.color, marginBottom: 4 }}>{k!.mark} {k!.label}</div>
                {n.label}
              </div>
            )}
          </div>
        );
      })}
    </AbsoluteFill>
  );
};

const SearchScene = () => {
  const f = useCurrentFrame();
  const q1 = "pricing", q2 = "dinner recipes";
  const phase2 = f > 100;
  const query = phase2 ? q2.slice(0, Math.floor(interpolate(f, [104, 130], [0, q2.length], clamp))) : q1.slice(0, Math.floor(interpolate(f, [10, 28], [0, q1.length], clamp)));
  return (
    <AbsoluteFill>
      <Caption sub="Matches by meaning, not exact words — and says so when there's nothing.">Find it later.</Caption>
      <div style={{ position: "absolute", left: 120, right: 120, top: 340 }}>
        <div style={{ fontFamily: body, fontSize: 40, color: C.ink, background: C.panel, border: `2px solid ${C.blue}`, borderRadius: 18, padding: "20px 28px" }}>
          <span style={{ color: C.muted }}>⌕ </span>{query}<span style={{ opacity: f % 20 < 10 ? 1 : 0 }}>|</span>
        </div>
        {!phase2 ? (
          <div style={{ marginTop: 26, display: "flex", flexDirection: "column", gap: 18, opacity: fade(f, 34, 54) }}>
            <Chip kind="decision" text="We decided to price it at $9 a month" />
            <Chip kind="question" text="Should I offer a free tier?" />
            <Chip kind="task" text="Draft the pricing page tonight" />
          </div>
        ) : (
          <div style={{ marginTop: 40, fontFamily: body, fontSize: 36, color: C.muted, opacity: fade(f, 134, 150) }}>Nothing about that yet.</div>
        )}
      </div>
    </AbsoluteFill>
  );
};

const Local = () => {
  const f = useCurrentFrame();
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", gap: 26 }}>
      <div style={{ fontFamily: display, fontWeight: 800, fontSize: 104, letterSpacing: -3, color: C.ink, opacity: fade(f, 0, 16), textAlign: "center", lineHeight: 1.05 }}>
        All of it runs<br /><span style={{ color: C.teal }}>on your Mac.</span>
      </div>
      <div style={{ fontFamily: body, fontSize: 36, color: C.muted, opacity: fade(f, 20, 36) }}>No account. No cloud. Free and open source.</div>
      <div style={{ fontFamily: mono, fontSize: 34, color: C.ink, padding: "16px 34px", borderRadius: 16, background: C.blue, opacity: fade(f, 40, 56) }}>abhishek085.github.io/Nodemind</div>
    </AbsoluteFill>
  );
};

export const Launch = () => (
  <AbsoluteFill style={{ background: `radial-gradient(1200px 700px at 70% 0%, #172445 0%, ${C.bg} 60%)` }}>
    <Sequence from={0} durationInFrames={S(3.5)}><Title /></Sequence>
    <Sequence from={S(3.5)} durationInFrames={S(5)}><Ramble /></Sequence>
    <Sequence from={S(8.5)} durationInFrames={S(5.5)}><Sort /></Sequence>
    <Sequence from={S(14)} durationInFrames={S(4.5)}><MapScene /></Sequence>
    <Sequence from={S(18.5)} durationInFrames={S(4.5)}><SearchScene /></Sequence>
    <Sequence from={S(23)} durationInFrames={S(3)}><Local /></Sequence>
  </AbsoluteFill>
);
