use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
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
        persona: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, TS)]
#[ts(export)]
pub struct FrontendCapabilities {
    pub protocol_version: u32,
    pub audio: Option<AudioCapabilities>,
    pub tools: Vec<String>,
    pub avatar_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub struct AudioCapabilities {
    pub input_pcm: bool,
    pub output_pcm: bool,
    pub sample_rates_hz: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerEvent {
    Hello {
        protocol_version: u32,
        binary_audio: bool,
    },
    SessionStarted {
        session_id: String,
        model: Option<String>,
    },
    SessionEnded,
    Phase {
        phase: String,
    },
    TurnStarted {
        turn_id: String,
        character: Option<String>,
        persona: Option<String>,
    },
    AssistantDelta {
        turn_id: String,
        delta: String,
    },
    UserTranscript {
        text: String,
        #[serde(rename = "final")]
        final_: bool,
        confidence: Option<f32>,
    },
    TurnCompleted {
        turn_id: String,
    },
    AudioReset {
        reason: Option<String>,
    },
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
    FrontendToolsNegotiated {
        tools: Vec<String>,
    },
    FrontendToolCall {
        name: String,
        arguments: Value,
    },
    FrontendToolRejected {
        name: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ServerEvent;

    #[test]
    fn session_started_serializes_current_model() {
        let event = ServerEvent::SessionStarted {
            session_id: "session-1".to_string(),
            model: Some("qwen3.6-35b".to_string()),
        };

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "type": "session_started",
                "session_id": "session-1",
                "model": "qwen3.6-35b"
            })
        );
    }
}
