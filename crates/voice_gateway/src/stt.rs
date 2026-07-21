use anyhow::{bail, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    frame::{Frame, FrameEnvelope, SessionId, SttFrame, VadFrame},
    trace::{TraceWriter, EVENT_STT_FINAL, EVENT_STT_PARTIAL},
};

#[async_trait]
pub trait SttAdapter: Send {
    async fn send_pcm(&mut self, session_id: SessionId, pcm: Bytes) -> Result<FrameEnvelope>;
    async fn reset(&mut self) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
    fn try_next_frame(&mut self) -> Option<FrameEnvelope>;
    async fn next_frame(&mut self) -> Option<FrameEnvelope>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParakeetSileroConfig {
    pub url: String,
    pub input_sample_rate_hz: u32,
    pub backend: String,
}

impl Default for ParakeetSileroConfig {
    fn default() -> Self {
        Self {
            url: "ws://127.0.0.1:8796/ws".to_string(),
            input_sample_rate_hz: 24_000,
            backend: "parakeet-silero".to_string(),
        }
    }
}

pub struct ParakeetSileroSttAdapter {
    config: ParakeetSileroConfig,
    sender: Option<mpsc::Sender<Message>>,
    events: Option<mpsc::Receiver<FrameEnvelope>>,
    trace: Option<TraceWriter>,
}

impl ParakeetSileroSttAdapter {
    pub fn new(config: ParakeetSileroConfig) -> Self {
        Self {
            config,
            sender: None,
            events: None,
            trace: None,
        }
    }

    pub fn with_trace(mut self, trace: TraceWriter) -> Self {
        self.trace = Some(trace);
        self
    }

    pub async fn connect(&mut self, session_id: SessionId) -> Result<()> {
        if self.sender.is_some() {
            return Ok(());
        }
        ensure_supported_stt_backend(&self.config.backend)?;
        let (ws, _) = connect_async(&self.config.url).await?;
        let (mut write, mut read) = ws.split();
        let (tx, mut rx) = mpsc::channel::<Message>(128);
        let (event_tx, event_rx) = mpsc::channel::<FrameEnvelope>(128);
        let reader_session_id = session_id.clone();

        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if write.send(message).await.is_err() {
                    return;
                }
            }
        });

        tokio::spawn(async move {
            while let Some(message) = read.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                match parakeet_message_to_frames(&reader_session_id, &text) {
                    Ok(frames) => {
                        for frame in frames {
                            if event_tx.send(frame).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        let frame = FrameEnvelope::new(
                            reader_session_id.clone(),
                            Frame::Stt(SttFrame::Error {
                                message: err.to_string(),
                            }),
                        );
                        if event_tx.send(frame).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        tx.send(Message::Text(
            json!({ "type": "setlanguage", "lang": "en" }).to_string(),
        ))
        .await?;
        tx.send(Message::Text(json!({ "type": "getstatus" }).to_string()))
            .await?;

        self.sender = Some(tx);
        self.events = Some(event_rx);
        Ok(())
    }
}

#[async_trait]
impl SttAdapter for ParakeetSileroSttAdapter {
    async fn send_pcm(&mut self, session_id: SessionId, pcm: Bytes) -> Result<FrameEnvelope> {
        self.connect(session_id.clone()).await?;
        if let Some(sender) = &self.sender {
            sender
                .send(Message::Binary(pcm16le_to_f32le(&pcm)?.to_vec()))
                .await?;
        }
        Ok(FrameEnvelope::new(
            session_id,
            Frame::Vad(VadFrame::FrontendHint {
                speaking: true,
                confidence: None,
            }),
        ))
    }

    async fn reset(&mut self) -> Result<()> {
        if let Some(sender) = &self.sender {
            sender
                .send(Message::Text(json!({ "type": "stop" }).to_string()))
                .await?;
        }
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.sender = None;
        self.events = None;
        Ok(())
    }

    fn try_next_frame(&mut self) -> Option<FrameEnvelope> {
        let frame = self.events.as_mut()?.try_recv().ok()?;
        if let Some(trace) = &self.trace {
            if let Some((event, data)) = stt_trace_event(&frame) {
                let _ = trace.event(event, data);
            }
        }
        Some(frame)
    }

    async fn next_frame(&mut self) -> Option<FrameEnvelope> {
        let frame = self.events.as_mut()?.recv().await?;
        if let Some(trace) = &self.trace {
            if let Some((event, data)) = stt_trace_event(&frame) {
                let _ = trace.event(event, data);
            }
        }
        Some(frame)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParakeetMessage {
    #[serde(rename = "type")]
    message_type: String,
    word: Option<String>,
    text: Option<String>,
    message: Option<String>,
}

pub fn parakeet_message_to_frames(
    session_id: &SessionId,
    text: &str,
) -> Result<Vec<FrameEnvelope>> {
    let message: ParakeetMessage = serde_json::from_str(text)?;
    Ok(match message.message_type.as_str() {
        "word" => message
            .word
            .filter(|word| !word.trim().is_empty())
            .map(|word| {
                vec![FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Stt(SttFrame::Partial {
                        text: word,
                        confidence: None,
                    }),
                )]
            })
            .unwrap_or_default(),
        "interim" => message
            .text
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Stt(SttFrame::Partial {
                        text,
                        confidence: None,
                    }),
                )]
            })
            .unwrap_or_default(),
        "final" => message
            .text
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![
                    FrameEnvelope::new(session_id.clone(), Frame::Vad(VadFrame::SpeechStopped)),
                    FrameEnvelope::new(
                        session_id.clone(),
                        Frame::Stt(SttFrame::Final {
                            text,
                            confidence: None,
                        }),
                    ),
                ]
            })
            .unwrap_or_default(),
        "status" => status_to_vad_frames(session_id, message.message.as_deref()),
        "error" => vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Stt(SttFrame::Error {
                message: message
                    .message
                    .unwrap_or_else(|| "Parakeet/Silero STT error".to_string()),
            }),
        )],
        _ => Vec::new(),
    })
}

fn status_to_vad_frames(session_id: &SessionId, message: Option<&str>) -> Vec<FrameEnvelope> {
    let message = message.unwrap_or_default().to_lowercase();
    if message.contains("speech start") {
        vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Vad(VadFrame::SpeechStarted),
        )]
    } else if message.contains("speech end") || message.contains("local silence detected") {
        vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Vad(VadFrame::SpeechStopped),
        )]
    } else {
        Vec::new()
    }
}

/// Transport for the eaRS STT backend (ADR 0007). Both variants share the same
/// [`ears_message_to_frames`] mapping; only process lifecycle differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EarsTransport {
    /// Gateway spawns and owns a local `ears-server` on loopback (low latency).
    Managed,
    /// Gateway connects to an already-running `ears-server` (split services).
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarsConfig {
    pub transport: EarsTransport,
    /// eaRS engine selector: `kyutai`, `parakeet-rs`, or `transcribe-cpp`.
    /// Availability is negotiated with the server; unknown engines fall back
    /// to the server default.
    pub engine: String,
    /// Used only when `transport == Remote`.
    pub url: String,
    pub input_sample_rate_hz: u32,
}

impl Default for EarsConfig {
    fn default() -> Self {
        Self {
            transport: EarsTransport::Managed,
            engine: "parakeet-rs".to_string(),
            url: "ws://127.0.0.1:8796/ws".to_string(),
            input_sample_rate_hz: 24_000,
        }
    }
}

#[derive(Debug, Deserialize)]
struct EarsMessage {
    #[serde(rename = "type")]
    message_type: String,
    word: Option<String>,
    text: Option<String>,
    active: Option<bool>,
}

/// Map an eaRS WebSocket message (internally tagged, lowercase) onto canonical
/// Foxline frames. The `speech` boundary event is authoritative for turn logic,
/// so `pause` is intentionally ignored to avoid double-firing `SpeechStopped`.
pub fn ears_message_to_frames(session_id: &SessionId, text: &str) -> Result<Vec<FrameEnvelope>> {
    let message: EarsMessage = serde_json::from_str(text)?;
    Ok(match message.message_type.as_str() {
        "word" => message
            .word
            .filter(|word| !word.trim().is_empty())
            .map(|word| {
                vec![FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Stt(SttFrame::Partial {
                        text: word,
                        confidence: None,
                    }),
                )]
            })
            .unwrap_or_default(),
        "final" => message
            .text
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![
                    FrameEnvelope::new(session_id.clone(), Frame::Vad(VadFrame::SpeechStopped)),
                    FrameEnvelope::new(
                        session_id.clone(),
                        Frame::Stt(SttFrame::Final {
                            text,
                            confidence: None,
                        }),
                    ),
                ]
            })
            .unwrap_or_default(),
        "speech" => match message.active {
            Some(true) => vec![FrameEnvelope::new(
                session_id.clone(),
                Frame::Vad(VadFrame::SpeechStarted),
            )],
            Some(false) => vec![FrameEnvelope::new(
                session_id.clone(),
                Frame::Vad(VadFrame::SpeechStopped),
            )],
            None => Vec::new(),
        },
        // `pause`, `status`, `languagechanged`, `enginechanged`: not turn-authoritative.
        _ => Vec::new(),
    })
}

pub fn stt_trace_event(frame: &FrameEnvelope) -> Option<(&'static str, serde_json::Value)> {
    match &frame.frame {
        Frame::Stt(SttFrame::Partial { text, .. }) => Some((
            EVENT_STT_PARTIAL,
            json!({ "text_chars": text.chars().count() }),
        )),
        Frame::Stt(SttFrame::Final { text, .. }) => Some((
            EVENT_STT_FINAL,
            json!({ "text_chars": text.chars().count() }),
        )),
        _ => None,
    }
}

fn pcm16le_to_f32le(pcm: &[u8]) -> Result<Bytes> {
    if pcm.len() % 2 != 0 {
        bail!("PCM16 input must have an even byte length");
    }
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for chunk in pcm.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(Bytes::from(out))
}

pub fn ensure_supported_stt_backend(name: &str) -> Result<()> {
    match name {
        "parakeet-silero" | "ears" => Ok(()),
        "parakeet" | "silero" | "whisper" | "nemotron" => {
            bail!("STT backend {name} is reserved but not implemented yet")
        }
        other => bail!("unsupported STT backend {other}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::frame::{Frame, SessionId, SttFrame, VadFrame};

    use super::{
        ears_message_to_frames, ensure_supported_stt_backend, parakeet_message_to_frames,
        pcm16le_to_f32le, stt_trace_event,
    };

    #[test]
    fn ears_backend_is_supported() {
        assert!(ensure_supported_stt_backend("ears").is_ok());
    }

    #[test]
    fn ears_speech_events_map_to_vad_boundaries() {
        let session_id = SessionId::new();
        let start = ears_message_to_frames(
            &session_id,
            r#"{"type":"speech","active":true,"timestamp":1.0}"#,
        )
        .unwrap();
        assert!(matches!(
            start[0].frame,
            Frame::Vad(VadFrame::SpeechStarted)
        ));
        let stop = ears_message_to_frames(
            &session_id,
            r#"{"type":"speech","active":false,"timestamp":2.0}"#,
        )
        .unwrap();
        assert!(matches!(stop[0].frame, Frame::Vad(VadFrame::SpeechStopped)));
    }

    #[test]
    fn ears_final_stops_speech_and_emits_final_transcript() {
        let session_id = SessionId::new();
        let frames =
            ears_message_to_frames(&session_id, r#"{"type":"final","text":"done","words":[]}"#)
                .unwrap();
        assert!(matches!(
            frames[0].frame,
            Frame::Vad(VadFrame::SpeechStopped)
        ));
        assert!(matches!(
            frames[1].frame,
            Frame::Stt(SttFrame::Final { .. })
        ));
    }

    #[test]
    fn ears_pause_and_status_are_not_turn_authoritative() {
        let session_id = SessionId::new();
        assert!(
            ears_message_to_frames(&session_id, r#"{"type":"pause","timestamp":1.0}"#)
                .unwrap()
                .is_empty()
        );
        assert!(ears_message_to_frames(
            &session_id,
            r#"{"type":"status","paused":false,"vad":true,"timestamps":false}"#
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn maps_word_and_interim_to_partial_transcripts() {
        let session_id = SessionId::new();
        let word =
            parakeet_message_to_frames(&session_id, r#"{"type":"word","word":"hello"}"#).unwrap();
        let interim =
            parakeet_message_to_frames(&session_id, r#"{"type":"interim","text":"hello there"}"#)
                .unwrap();

        assert!(matches!(
            &word[0].frame,
            Frame::Stt(SttFrame::Partial { text, .. }) if text == "hello"
        ));
        assert!(matches!(
            &interim[0].frame,
            Frame::Stt(SttFrame::Partial { text, .. }) if text == "hello there"
        ));
    }

    #[test]
    fn maps_final_to_speech_stop_and_final_transcript() {
        let session_id = SessionId::new();
        let frames =
            parakeet_message_to_frames(&session_id, r#"{"type":"final","text":"done"}"#).unwrap();

        assert!(matches!(
            frames[0].frame,
            Frame::Vad(VadFrame::SpeechStopped)
        ));
        assert!(matches!(
            &frames[1].frame,
            Frame::Stt(SttFrame::Final { text, .. }) if text == "done"
        ));
    }

    #[test]
    fn maps_status_speech_boundaries_to_vad_frames() {
        let session_id = SessionId::new();
        let start = parakeet_message_to_frames(
            &session_id,
            r#"{"type":"status","message":"speech start #1"}"#,
        )
        .unwrap();
        let stop = parakeet_message_to_frames(
            &session_id,
            r#"{"type":"status","message":"speech end 1.20s"}"#,
        )
        .unwrap();

        assert!(matches!(
            start[0].frame,
            Frame::Vad(VadFrame::SpeechStarted)
        ));
        assert!(matches!(stop[0].frame, Frame::Vad(VadFrame::SpeechStopped)));
    }

    #[test]
    fn maps_error_and_trace_events() {
        let session_id = SessionId::new();
        let err =
            parakeet_message_to_frames(&session_id, r#"{"type":"error","message":"bad"}"#).unwrap();

        assert!(matches!(
            &err[0].frame,
            Frame::Stt(SttFrame::Error { message }) if message == "bad"
        ));

        let partial =
            parakeet_message_to_frames(&session_id, r#"{"type":"word","word":"hello"}"#).unwrap();
        assert_eq!(stt_trace_event(&partial[0]).unwrap().0, "stt_partial");
    }

    #[test]
    fn backend_selection_names_are_explicit() {
        assert!(ensure_supported_stt_backend("parakeet-silero").is_ok());
        assert!(ensure_supported_stt_backend("nemotron").is_err());
        assert!(ensure_supported_stt_backend("unknown").is_err());
    }

    #[test]
    fn converts_gateway_pcm16_to_parakeet_float32_wire_format() {
        let bytes = pcm16le_to_f32le(&[
            0x00, 0x00, // 0
            0xff, 0x7f, // i16::MAX
            0x00, 0x80, // i16::MIN
        ])
        .unwrap();

        let samples = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect::<Vec<_>>();

        assert_eq!(samples[0], 0.0);
        assert!((samples[1] - 0.9999695).abs() < 0.000001);
        assert_eq!(samples[2], -1.0);
    }
}
