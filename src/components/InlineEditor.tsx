import React, { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface InlineEditorProps {
  nodeId: string;
  label: string;
  onSave?: (newLabel: string) => void;
  children: (startEdit: () => void) => React.ReactNode;
}

/**
 * Inline editor for correcting LLM-extracted entity labels (e.g. mispronunciations).
 * Wraps any child element. Call startEdit() (passed via render prop) to open the editor.
 * On save, calls invoke("rename_graph_node", { nodeId, newLabel }) and calls onSave callback.
 */
const InlineEditor: React.FC<InlineEditorProps> = ({ nodeId, label, onSave, children }) => {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(label);
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const startEdit = () => {
    setValue(label);
    setEditing(true);
  };

  const cancel = () => {
    setEditing(false);
    setValue(label);
  };

  const save = async () => {
    const trimmed = value.trim();
    if (!trimmed || trimmed === label) {
      cancel();
      return;
    }
    setSaving(true);
    try {
      await invoke("rename_graph_node", { nodeId, newLabel: trimmed });
      onSave?.(trimmed);
      setEditing(false);
    } catch {
      // silently cancel on error so UI doesn't break
      cancel();
    } finally {
      setSaving(false);
    }
  };

  const handleKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") save();
    if (e.key === "Escape") cancel();
  };

  if (editing) {
    return (
      <div className="inline-editor-wrap" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="inline-editor-input"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKey}
          disabled={saving}
          placeholder="Correct the name..."
          maxLength={80}
        />
        <button
          className="inline-editor-btn inline-editor-save"
          onClick={save}
          disabled={saving || !value.trim()}
        >
          {saving ? "…" : "✓"}
        </button>
        <button className="inline-editor-btn inline-editor-cancel" onClick={cancel} disabled={saving}>
          ✕
        </button>
      </div>
    );
  }

  return <>{children(startEdit)}</>;
};

export default InlineEditor;
