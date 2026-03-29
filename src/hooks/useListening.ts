import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ListeningStatus } from "../types";

export function useListening() {
  const [status, setStatus] = useState<ListeningStatus>("off");
  const [transcript, setTranscript] = useState("");
  const [language, setLanguageState] = useState<"en" | "hi">("en");
  const [processingFinalNote, setProcessingFinalNote] = useState(false);
  const unlisten = useRef<UnlistenFn[]>([]);

  // On mount, load today's persisted transcript from the DB so previous
  // session chunks are visible even before the user hits Record.
  useEffect(() => {
    invoke<string>("get_today_transcript")
      .then((t) => { if (t) setTranscript(t); })
      .catch(() => {});
  }, []);

  useEffect(() => {
    // Poll transcript every 1.5s when listening
    const id = setInterval(async () => {
      try {
        const t: string = await invoke("get_latest_transcript");
        if (t) setTranscript(t);
        const listening: boolean = await invoke("is_listening");
        if (!listening && status === "listening") setStatus("idle");
      } catch {}
    }, 1500);

    return () => clearInterval(id);
  }, [status]);

  useEffect(() => {
    let mounted = true;

    const setup = async () => {
      const u0 = await listen<string>("transcript-chunk", (event) => {
        if (!mounted) return;
        const text = String(event.payload ?? "").trim();
        if (!text) return;
        setTranscript((prev) => (prev ? `${prev} ${text}` : text));
      });
      unlisten.current.push(u0);

      const u1 = await listen("listening-idle", () => {
        if (mounted) setStatus("idle");
      });
      unlisten.current.push(u1);

      const u2 = await listen("final-note-processing-started", () => {
        if (mounted) setProcessingFinalNote(true);
      });
      unlisten.current.push(u2);

      const u3 = await listen("final-note-processing-finished", () => {
        if (mounted) setProcessingFinalNote(false);
      });
      unlisten.current.push(u3);
    };

    setup();
    return () => {
      mounted = false;
      unlisten.current.forEach((u) => u());
    };
  }, []);

  const toggleListening = async () => {
    if (status === "listening") {
      await invoke("stop_listening");
      setStatus("off");
    } else {
      await invoke("start_listening");
      setStatus("listening");
    }
  };

  const setLanguage = async (lang: "en" | "hi") => {
    setLanguageState(lang);
    await invoke("set_language", { language: lang });
  };

  const clearTranscript = async () => {
    await invoke("clear_transcript");
    setTranscript("");
  };

  return { status, transcript, language, processingFinalNote, toggleListening, setLanguage, clearTranscript };
}
