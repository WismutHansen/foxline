use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameEnvelope {
    pub id: Uuid,
    pub session_id: SessionId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub frame: Frame,
}

impl FrameEnvelope {
    pub fn new(session_id: SessionId, frame: Frame) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            created_at: OffsetDateTime::now_utc(),
            frame,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "family", content = "payload", rename_all = "snake_case")]
pub enum Frame {
    Audio(AudioFrame),
    Vad(VadFrame),
    Stt(SttFrame),
    Turn(TurnFrame),
    Brain(BrainFrame),
    Tts(TtsFrame),
    FrontendTool(FrontendToolFrame),
    AvatarAction(AvatarActionFrame),
    Lifecycle(LifecycleFrame),
    Error(ErrorFrame),
    Metrics(MetricsFrame),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioFrame {
    InputPcm {
        sample_rate_hz: u32,
        channels: u16,
        bytes: Bytes,
    },
    OutputPcm {
        sample_rate_hz: u32,
        channels: u16,
        bytes: Bytes,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VadFrame {
    FrontendHint {
        speaking: bool,
        confidence: Option<f32>,
    },
    SpeechStarted,
    SpeechStopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SttFrame {
    Partial {
        text: String,
        confidence: Option<f32>,
    },
    Final {
        text: String,
        confidence: Option<f32>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnFrame {
    UserStarted,
    UserCommitted { text: String },
    Interrupted { reason: InterruptReason },
    AssistantStarted,
    AssistantFinished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    FrontendBargeIn,
    VadSpeechStart,
    UserCancel,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrainFrame {
    RequestStart { text: String },
    FirstToken,
    TextDelta { text: String },
    ToolCall { name: String, arguments: Value },
    ToolResult { call_id: String, result: Value },
    Done,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TtsFrame {
    RequestStart { text: String },
    AudioStart { sample_rate_hz: u32, channels: u16 },
    AudioChunk { bytes: Bytes },
    AudioDone,
    Cancel,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendToolFrame {
    Negotiated { tools: Vec<String> },
    Call { name: String, arguments: Value },
    Result { call_id: String, result: Value },
    Rejected { name: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AvatarActionFrame {
    SetState { state: String },
    SetExpression { expression: String },
    Focus { target: String },
    PlayAnimation { name: String },
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleFrame {
    SessionStarted,
    SessionEnded,
    CapabilityDeclared { profile: Value },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorFrame {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsFrame {
    pub event: String,
    pub elapsed_ms: Option<u64>,
    pub data: Value,
}
