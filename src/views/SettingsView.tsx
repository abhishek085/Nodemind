import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function SettingsView() {
  const [ollamaStatus, setOllamaStatus] = useState<{
    available: boolean;
    models: string[];
  } | null>(null);
  const [llmModel, setLlmModelState] = useState("qwen3.5:9b");
  const [checking, setChecking] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [modelLoaded, setModelLoaded] = useState(false);
  const [loadingModel, setLoadingModel] = useState(false);

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  const checkOllama = async () => {
    setChecking(true);
    try {
      const res: { available: boolean; models: string[] } = await invoke(
        "check_ollama_status",
      );
      setOllamaStatus(res);
    } catch {
      setOllamaStatus({ available: false, models: [] });
    } finally {
      setChecking(false);
    }
  };

  const loadModel = async () => {
    try {
      const m: string = await invoke("get_llm_model");
      setLlmModelState(m);
    } catch {}
  };

  const pollModelLoaded = async () => {
    try {
      const loaded: boolean = await invoke("is_model_loaded");
      setModelLoaded(loaded);
    } catch {}
  };

  useEffect(() => {
    checkOllama();
    loadModel();
    pollModelLoaded();
  }, []);

  const applyAndLoadModel = async (modelName: string) => {
    if (!modelName || loadingModel || saveStatus === "saving") {
      return;
    }

    setLlmModelState(modelName);
    setSaveStatus("saving");
    setLoadingModel(true);

    try {
      const ok: boolean = await invoke("set_llm_model", { model: modelName });
      if (!ok) {
        setSaveStatus("error");
        setLoadingModel(false);
        return;
      }

      setSaveStatus("saved");
      let loaded = false;
      for (let i = 0; i < 80; i++) {
        try {
          loaded = await invoke("is_model_loaded");
          setModelLoaded(loaded);
          if (loaded) {
            break;
          }
        } catch {
          break;
        }
        await sleep(1500);
      }
      setLoadingModel(false);
    } catch {
      setSaveStatus("error");
      setLoadingModel(false);
    } finally {
      setTimeout(() => setSaveStatus("idle"), 2500);
    }
  };

  const handleUnload = async () => {
    await invoke("unload_model_cmd");
    setModelLoaded(false);
  };

  const handleLoad = async () => {
    setLoadingModel(true);
    try {
      await invoke("load_model_cmd");
      // Poll until model is warm
      const poll = setInterval(async () => {
        try {
          const loaded: boolean = await invoke("is_model_loaded");
          setModelLoaded(loaded);
          if (loaded) {
            clearInterval(poll);
            setLoadingModel(false);
          }
        } catch {
          clearInterval(poll);
          setLoadingModel(false);
        }
      }, 1500);
      setTimeout(() => { clearInterval(poll); setLoadingModel(false); }, 120000);
    } catch {
      setLoadingModel(false);
    }
  };

  return (
    <div className="view-settings">
      <div className="view-header">
        <div>
          <h1 className="view-title">Settings</h1>
          <p className="view-subtitle">Configure Nodemind's local AI pipeline</p>
        </div>
      </div>

      {/* Ollama section */}
      <div className="card settings-card">
        <div className="settings-section-title">Local LLM (Ollama)</div>

        <div className="settings-row">
          <div className="settings-label">Status</div>
          <div className="settings-value">
            {ollamaStatus === null ? (
              <span className="hint-text">Checking…</span>
            ) : ollamaStatus.available ? (
              <span className="badge-ok">Connected</span>
            ) : (
              <span className="badge-warn">Not running</span>
            )}
            <button
              className="action-btn ml-1"
              onClick={checkOllama}
              disabled={checking}
            >
              {checking ? "…" : "Recheck"}
            </button>
          </div>
        </div>

        {ollamaStatus?.models && ollamaStatus.models.length > 0 && (
          <div className="settings-row">
            <div className="settings-label">Available models</div>
            <div className="settings-value">
              <div className="model-list">
                {ollamaStatus.models.map((m) => (
                  <button
                    key={m}
                    type="button"
                    className={`model-chip${llmModel === m ? " model-chip-active" : ""}`}
                    onClick={() => applyAndLoadModel(m)}
                    disabled={saveStatus === "saving" || loadingModel}
                    title="Select and load this model"
                  >
                    {m}
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}

        <div className="settings-row">
          <div className="settings-label">Active model</div>
          <div className="settings-value">
            <span className="code-inline">{llmModel || "Not selected"}</span>
            <span className="hint-text">Click an available model above to select and load it.</span>
          </div>
        </div>

        <div className="settings-row">
          <div className="settings-label">Model status</div>
          <div className="settings-value">
            {loadingModel ? (
              <span className="hint-text">Loading into memory…</span>
            ) : modelLoaded ? (
              <>
                <span className="badge-ok">Hot (in memory)</span>
                <button className="action-btn ml-1" onClick={handleUnload}>Unload</button>
              </>
            ) : (
              <>
                <span className="badge-warn">Not loaded</span>
                <button className="action-btn ml-1" onClick={handleLoad}>Load</button>
              </>
            )}
          </div>
        </div>

        {!ollamaStatus?.available && (
          <div className="settings-notice">
            <strong>To enable LLM features:</strong>
            <ol className="setup-steps">
              <li>
                Install Ollama:{" "}
                <span className="code-inline">brew install ollama</span>
              </li>
              <li>
                Start it: <span className="code-inline">ollama serve</span>
              </li>
              <li>
                Pull a model:{" "}
                <span className="code-inline">ollama pull llama3</span>
              </li>
            </ol>
          </div>
        )}
      </div>

      <div className="settings-notice">
        <strong>Recommended setup for best performance:</strong>
        <ol className="setup-steps">
          <li>At least 16 GB RAM</li>
          <li>Apple Silicon M2 or newer</li>
          <li>At least an 8B-parameter local model</li>
        </ol>
      </div>

      {/* STT section */}
      <div className="card settings-card">
        <div className="settings-section-title">Speech-to-Text (Whisper)</div>
        <div className="settings-row">
          <div className="settings-label">Backend</div>
          <div className="settings-value">
            <span className="badge-ok">Local (CoreML + GGML)</span>
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-label">Model</div>
          <div className="settings-value">
            <span className="code-inline">ggml-small.bin</span>
            <span className="hint-text ml-1">
              + CoreML encoder (Apple Neural Engine)
            </span>
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-label">Languages</div>
          <div className="settings-value">English · Hindi/Hinglish → English</div>
        </div>
      </div>

      {/* Privacy */}
      <div className="card settings-card">
        <div className="settings-section-title">Privacy</div>
        <div className="privacy-list">
          <div className="privacy-item">
            <span className="privacy-icon">🔒</span>
            All processing is 100% local — no data leaves your Mac
          </div>
          <div className="privacy-item">
            <span className="privacy-icon">🗑️</span>
            Raw audio is discarded immediately after transcription
          </div>
          <div className="privacy-item">
            <span className="privacy-icon">💾</span>
            Data stored in{" "}
            <span className="code-inline">
              ~/Library/Application Support/com.nokast.nodemind/nodemind.db
            </span>
          </div>
          <div className="privacy-item">
            <span className="privacy-icon">📡</span>
            No telemetry, no analytics, no network calls
          </div>
        </div>
      </div>

      {/* Direct command guide */}
      <div className="card settings-card">
        <div className="settings-section-title">Voice Command Examples</div>
        <div className="command-examples">
          {[
            ["Start meeting", '"I am taking a meeting with Raghav about Nokast"'],
            ["End meeting", '"meeting over"'],
            ["Create task", '"remind me tomorrow I need to call the bank"'],
            ["Set goal", '"my goal is to ship the app by May 30"'],
            ["Block time", '"block two hours this evening for deep work"'],
          ].map(([intent, example]) => (
            <div key={intent} className="command-row">
              <span className="command-intent">{intent}</span>
              <span className="command-example">{example}</span>
            </div>
          ))}
        </div>
      </div>

      <div className="app-version">Nodemind v0.1.0 · Built with Tauri + Whisper + Ollama</div>
    </div>
  );
}
