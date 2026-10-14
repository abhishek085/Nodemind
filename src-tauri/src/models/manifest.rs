//! The static model manifest.
//!
//! Sizes/URLs verified against HuggingFace (see `scripts/fetch-models.sh` for
//! the same list, used to pre-bake models into the app bundle at build time).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelKey {
    /// whisper.cpp small — speech → text (multilingual, handles Hinglish)
    Whisper,
    /// Llama 3.2 1B Instruct (Q4_K_M) — checklist cleanup, graph annotation
    Llm,
    /// EmbeddingGemma 2 (Q8_0) — semantic similarity / vector store
    Embed,
}

impl ModelKey {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKey::Whisper => "whisper",
            ModelKey::Llm => "llm",
            ModelKey::Embed => "embed",
        }
    }

    /// Local filename inside the models directory.
    pub fn filename(self) -> &'static str {
        match self {
            ModelKey::Whisper => "ggml-small.bin",
            ModelKey::Llm => "llama-3.2-1b-instruct-q4_k_m.gguf",
            ModelKey::Embed => "embeddinggemma-2-q8_0.gguf",
        }
    }
}

impl fmt::Display for ModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub struct ModelDef {
    pub key: ModelKey,
    /// Human label for the setup screen.
    pub label: &'static str,
    /// Local filename (see `ModelKey::filename`).
    pub filename: &'static str,
    /// Direct HuggingFace download URL.
    pub url: &'static str,
    /// Expected sha256 (lowercase hex), verified after download.
    pub sha256: Option<&'static str>,
    /// Expected size, used for progress bars before the header arrives.
    pub expected_bytes: u64,
}

pub const MANIFEST: &[ModelDef] = &[
    ModelDef {
        key: ModelKey::Whisper,
        label: "Whisper · speech to text",
        filename: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        sha256: Some(
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        ),
        expected_bytes: 487_601_967,
    },
    ModelDef {
        key: ModelKey::Llm,
        label: "Llama 3.2 1B · checklist + graph",
        filename: "llama-3.2-1b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        sha256: Some(
            "6f85a640a97cf2bf5b8e764087b1e83da0fdb51d7c9fab7d0fece9385611df83",
        ),
        expected_bytes: 807_694_464,
    },
    ModelDef {
        key: ModelKey::Embed,
        label: "EmbeddingGemma 2 · memory + similarity",
        filename: "embeddinggemma-2-q8_0.gguf",
        url: "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/embeddinggemma-2-Q8_0.gguf",
        sha256: Some(
            "2188ac1deca4b77dffefd603c2776a9d76d9d74ec01841392982ebb840b09135",
        ),
        expected_bytes: 309_855_456,
    },
];

/// CPU architecture of the host, for the UI ("Apple Silicon" vs "Intel").
pub fn host_arch() -> String {
    let arch = std::env::consts::ARCH;
    match arch {
        "aarch64" => "apple-silicon".to_string(),
        "x86_64" => "intel".to_string(),
        other => other.to_string(),
    }
}
