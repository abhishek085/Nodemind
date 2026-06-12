use serde::Deserialize;
use std::sync::OnceLock;

/// The default prompts embedded at compile time from `resources/prompts.toml`.
/// Edit that file and recompile to update prompts without touching logic code.
const DEFAULT_TOML: &str = include_str!("../resources/prompts.toml");

static PROMPTS: OnceLock<Prompts> = OnceLock::new();

#[derive(Debug, Deserialize, Clone)]
pub struct SttPrompts {
    pub hindi_initial_prompt: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ParseCommandPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SummarizeTranscriptPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MeetingSummaryPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DailySuggestionsPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SignalClassifierPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FogSignalsOnlyPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FiveMinExtractPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskRagJudgePrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GoalHorizonClassifierPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Prompts {
    pub stt: SttPrompts,
    pub parse_command: ParseCommandPrompts,
    pub summarize_transcript: SummarizeTranscriptPrompts,
    pub meeting_summary: MeetingSummaryPrompts,
    pub daily_suggestions: DailySuggestionsPrompts,
    pub signal_classifier: SignalClassifierPrompts,
    pub fog_signals_only: FogSignalsOnlyPrompts,
    pub five_min_extract: FiveMinExtractPrompts,
    pub task_rag_judge: TaskRagJudgePrompts,
    pub goal_horizon_classifier: GoalHorizonClassifierPrompts,
    pub batch_entity_extract: BatchEntityExtractPrompts,
    pub drift_analysis: DriftAnalysisPrompts,
    pub realtime_fog: RealtimeFogPrompts,
    pub contradiction_surface: ContradictionSurfacePrompts,
    pub extraction_validator: ExtractionValidatorPrompts,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BatchEntityExtractPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DriftAnalysisPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RealtimeFogPrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ContradictionSurfacePrompts {
    pub template: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractionValidatorPrompts {
    pub template: String,
}

/// Returns the loaded prompts, initialising from the embedded default if needed.
pub fn get() -> &'static Prompts {
    PROMPTS.get_or_init(|| {
        toml::from_str(DEFAULT_TOML).expect("bundled prompts.toml must be valid TOML")
    })
}
