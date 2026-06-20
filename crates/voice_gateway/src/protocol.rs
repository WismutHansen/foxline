use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientControl {
    Hello {
        client: String,
        capabilities: FrontendCapabilities,
        debug_traces: Option<bool>,
    },
    StartSession {
        agent: String,
        workspace: String,
        loadout: Option<String>,
    },
    EndSession,
    VadHint {
        speaking: bool,
        confidence: Option<f32>,
    },
    Interrupt,
    FrontendToolResult {
        call_id: String,
        result: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FrontendCapabilities {
    pub protocol_version: u32,
    pub audio: Option<AudioCapabilities>,
    pub tools: Vec<String>,
    pub avatar_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioCapabilities {
    pub input_pcm: bool,
    pub output_pcm: bool,
    pub sample_rates_hz: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Hello {
        protocol_version: u32,
        binary_audio: bool,
    },
    SessionStarted {
        session_id: String,
    },
    SessionEnded,
    Error {
        code: String,
        message: String,
    },
    Trace {
        event: String,
        data: Value,
    },
    AvatarAction {
        action: Value,
    },
}
