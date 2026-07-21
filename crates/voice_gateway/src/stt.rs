use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::{net::TcpStream, process::Child, sync::mpsc};
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
/// [`EarsFrameMapper`] mapping; only process lifecycle differs.
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

/// Whether the managed transport adopted an already-running server or spawned
/// its own. Only a spawned server is ours to shut down.
#[derive(Debug)]
pub enum EarsServerHandle {
    Adopted,
    Spawned(Child),
}

impl EarsServerHandle {
    /// Terminate the server only if this handle spawned it. Adopting a
    /// pre-existing server must never kill it out from under its owner.
    pub async fn shutdown(&mut self) {
        if let EarsServerHandle::Spawned(child) = self {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

/// Split `host:port` from a `ws://host:port/path` URL for a raw TCP reachability
/// probe. Falls back to the loopback STT port when the URL lacks an authority.
pub fn ears_host_port(url: &str) -> (String, u16) {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(8796)),
        None => (authority.to_string(), 8796),
    }
}

/// True if a TCP listener already accepts connections at `host:port`.
pub async fn ears_server_reachable(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_millis(250), TcpStream::connect((host, port)),).await,
        Ok(Ok(_))
    )
}

/// Adopt a running `ears-server` if one is reachable at the configured address,
/// otherwise spawn one and wait until it accepts connections. The gateway only
/// owns (and later shuts down) a server it spawned itself.
pub async fn ensure_ears_server(config: &EarsConfig) -> Result<EarsServerHandle> {
    let (host, port) = ears_host_port(&config.url);

    if ears_server_reachable(&host, port).await {
        return Ok(EarsServerHandle::Adopted);
    }
    if config.transport == EarsTransport::Remote {
        bail!("no ears-server reachable at {host}:{port} (remote transport does not spawn)");
    }

    let bin =
        std::env::var("FOXLINE_EARS_SERVER_BIN").unwrap_or_else(|_| "ears-server".to_string());
    let child = tokio::process::Command::new(&bin)
        .arg("--bind")
        .arg(format!("{host}:{port}"))
        .arg("--engine")
        .arg(&config.engine)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn managed ears-server ({bin})"))?;

    for _ in 0..100 {
        if ears_server_reachable(&host, port).await {
            return Ok(EarsServerHandle::Spawned(child));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut handle = EarsServerHandle::Spawned(child);
    handle.shutdown().await;
    bail!("managed ears-server did not become ready at {host}:{port} within timeout")
}

/// STT adapter that talks to an eaRS server (ADR 0007). In managed transport it
/// adopts a running `ears-server` or spawns one; in remote transport it connects
/// to a configured address. Engine selection is forwarded to eaRS.
pub struct EarsSttAdapter {
    config: EarsConfig,
    server: Option<EarsServerHandle>,
    sender: Option<mpsc::Sender<Message>>,
    events: Option<mpsc::Receiver<FrameEnvelope>>,
    trace: Option<TraceWriter>,
}

impl EarsSttAdapter {
    pub fn new(config: EarsConfig) -> Self {
        Self {
            config,
            server: None,
            sender: None,
            events: None,
            trace: None,
        }
    }

    pub fn with_trace(mut self, trace: TraceWriter) -> Self {
        self.trace = Some(trace);
        self
    }

    async fn connect(&mut self, session_id: SessionId) -> Result<()> {
        if self.sender.is_some() {
            return Ok(());
        }
        self.server = Some(ensure_ears_server(&self.config).await?);

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
            let mut mapper = EarsFrameMapper::default();
            while let Some(message) = read.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                match mapper.map(&reader_session_id, &text) {
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

        // Request the configured engine; eaRS keeps its default if unavailable.
        tx.send(Message::Text(
            json!({ "type": "setengine", "engine": self.config.engine }).to_string(),
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
impl SttAdapter for EarsSttAdapter {
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
                .send(Message::Text(json!({ "type": "pause" }).to_string()))
                .await?;
        }
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.sender = None;
        self.events = None;
        if let Some(mut server) = self.server.take() {
            server.shutdown().await;
        }
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
struct EarsMessage {
    #[serde(rename = "type")]
    message_type: String,
    word: Option<String>,
    text: Option<String>,
    active: Option<bool>,
}

/// Maps eaRS WebSocket messages (internally tagged, lowercase) onto canonical
/// Foxline frames. eaRS streams individual `word` messages and only emits a
/// `final` at session teardown, so the per-turn transcript is accumulated here:
/// each `word` grows the current turn and emits a `Partial` with the full text
/// so far, and the end-of-turn `speech`/`active:false` boundary finalizes it
/// into `SpeechStopped` + `Final`. `pause` is ignored so it does not double-fire
/// with `speech`.
#[derive(Default)]
pub struct EarsFrameMapper {
    turn_text: String,
    in_turn: bool,
}

impl EarsFrameMapper {
    pub fn map(&mut self, session_id: &SessionId, text: &str) -> Result<Vec<FrameEnvelope>> {
        let message: EarsMessage = serde_json::from_str(text)?;
        Ok(match message.message_type.as_str() {
            "word" => {
                let Some(word) = message.word.filter(|word| !word.trim().is_empty()) else {
                    return Ok(Vec::new());
                };
                let mut frames = Vec::new();
                // eaRS does not reliably emit a speech-start for the very first
                // utterance, so the first word of a turn opens it.
                if !self.in_turn {
                    self.in_turn = true;
                    frames.push(FrameEnvelope::new(
                        session_id.clone(),
                        Frame::Vad(VadFrame::SpeechStarted),
                    ));
                }
                if !self.turn_text.is_empty() {
                    self.turn_text.push(' ');
                }
                self.turn_text.push_str(word.trim());
                frames.push(FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Stt(SttFrame::Partial {
                        text: self.turn_text.clone(),
                        confidence: None,
                    }),
                ));
                frames
            }
            "interim" => {
                let Some(text) = message.text.filter(|text| !text.trim().is_empty()) else {
                    return Ok(Vec::new());
                };
                let mut frames = Vec::new();
                if !self.in_turn {
                    self.in_turn = true;
                    frames.push(FrameEnvelope::new(
                        session_id.clone(),
                        Frame::Vad(VadFrame::SpeechStarted),
                    ));
                }
                // `Interim` is authoritative and revisable (`committed +
                // tentative`), so replace rather than append.
                self.turn_text = text.trim().to_string();
                frames.push(FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Stt(SttFrame::Partial {
                        text: self.turn_text.clone(),
                        confidence: None,
                    }),
                ));
                frames
            }
            "speech" => match message.active {
                Some(true) => {
                    if self.in_turn {
                        Vec::new()
                    } else {
                        self.in_turn = true;
                        vec![FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Vad(VadFrame::SpeechStarted),
                        )]
                    }
                }
                Some(false) => self.finalize(session_id),
                None => Vec::new(),
            },
            // eaRS `final` is a session-teardown transcript; prefer its cleaned
            // text over the accumulated words, then finalize any open turn.
            "final" => {
                if let Some(text) = message.text.filter(|text| !text.trim().is_empty()) {
                    self.turn_text = text.trim().to_string();
                }
                self.finalize(session_id)
            }
            // `pause`, `status`, `languagechanged`, `enginechanged`: not turn-authoritative.
            _ => Vec::new(),
        })
    }

    fn finalize(&mut self, session_id: &SessionId) -> Vec<FrameEnvelope> {
        self.in_turn = false;
        let text = std::mem::take(&mut self.turn_text);
        let mut frames = vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Vad(VadFrame::SpeechStopped),
        )];
        if !text.trim().is_empty() {
            frames.push(FrameEnvelope::new(
                session_id.clone(),
                Frame::Stt(SttFrame::Final {
                    text,
                    confidence: None,
                }),
            ));
        }
        frames
    }
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
    use crate::frame::{Frame, FrameEnvelope, SessionId, SttFrame, VadFrame};

    use super::{
        ensure_supported_stt_backend, parakeet_message_to_frames, pcm16le_to_f32le,
        stt_trace_event, EarsFrameMapper,
    };

    fn ears_frames(text: &str) -> Vec<FrameEnvelope> {
        EarsFrameMapper::default()
            .map(&SessionId::new(), text)
            .unwrap()
    }

    #[test]
    fn ears_backend_is_supported() {
        assert!(ensure_supported_stt_backend("ears").is_ok());
    }

    #[test]
    fn ears_host_port_parses_ws_url() {
        use super::ears_host_port;
        assert_eq!(
            ears_host_port("ws://127.0.0.1:8796/ws"),
            ("127.0.0.1".to_string(), 8796)
        );
        assert_eq!(
            ears_host_port("ws://stt.lan:9001"),
            ("stt.lan".to_string(), 9001)
        );
        assert_eq!(ears_host_port("localhost"), ("localhost".to_string(), 8796));
    }

    #[tokio::test]
    async fn ears_remote_transport_does_not_spawn_when_unreachable() {
        use super::{ensure_ears_server, EarsConfig, EarsTransport};
        let config = EarsConfig {
            transport: EarsTransport::Remote,
            // Reserved-for-documentation address that never accepts connections.
            url: "ws://192.0.2.1:9/ws".to_string(),
            ..EarsConfig::default()
        };
        let err = ensure_ears_server(&config).await.unwrap_err();
        assert!(err.to_string().contains("does not spawn"), "{err}");
    }

    #[test]
    fn ears_speech_active_false_finalizes_open_turn() {
        let session_id = SessionId::new();
        let mut mapper = EarsFrameMapper::default();
        // First word opens the turn (SpeechStarted) and grows the partial.
        let first = mapper
            .map(
                &session_id,
                r#"{"type":"word","word":"hello","start_time":0.0}"#,
            )
            .unwrap();
        assert!(matches!(
            first[0].frame,
            Frame::Vad(VadFrame::SpeechStarted)
        ));
        // End-of-turn boundary finalizes into SpeechStopped + Final.
        let end = mapper
            .map(
                &session_id,
                r#"{"type":"speech","active":false,"timestamp":2.0}"#,
            )
            .unwrap();
        assert!(matches!(end[0].frame, Frame::Vad(VadFrame::SpeechStopped)));
        assert!(matches!(
            &end[1].frame,
            Frame::Stt(SttFrame::Final { text, .. }) if text == "hello"
        ));
    }

    #[test]
    fn ears_words_accumulate_into_growing_partials() {
        let session_id = SessionId::new();
        let mut mapper = EarsFrameMapper::default();
        mapper
            .map(
                &session_id,
                r#"{"type":"word","word":"open","start_time":0.0}"#,
            )
            .unwrap();
        let second = mapper
            .map(
                &session_id,
                r#"{"type":"word","word":"the","start_time":0.1}"#,
            )
            .unwrap();
        // The partial carries the full turn so far, not just the last word.
        assert!(matches!(
            second.last().map(|f| &f.frame),
            Some(Frame::Stt(SttFrame::Partial { text, .. })) if text == "open the"
        ));
        let third = mapper
            .map(
                &session_id,
                r#"{"type":"word","word":"door","start_time":0.2}"#,
            )
            .unwrap();
        assert!(matches!(
            third.last().map(|f| &f.frame),
            Some(Frame::Stt(SttFrame::Partial { text, .. })) if text == "open the door"
        ));
        // A new turn starts fresh after finalization.
        mapper
            .map(
                &session_id,
                r#"{"type":"speech","active":false,"timestamp":1.0}"#,
            )
            .unwrap();
        let next = mapper
            .map(
                &session_id,
                r#"{"type":"word","word":"again","start_time":2.0}"#,
            )
            .unwrap();
        assert!(matches!(
            next.last().map(|f| &f.frame),
            Some(Frame::Stt(SttFrame::Partial { text, .. })) if text == "again"
        ));
    }

    #[test]
    fn ears_interim_replaces_preview_and_finalizes_authoritative_text() {
        let session_id = SessionId::new();
        let mut mapper = EarsFrameMapper::default();
        mapper
            .map(&session_id, r#"{"type":"interim","text":"hello wor"}"#)
            .unwrap();
        let revised = mapper
            .map(&session_id, r#"{"type":"interim","text":"hello world"}"#)
            .unwrap();
        assert!(matches!(
            revised.last().map(|f| &f.frame),
            Some(Frame::Stt(SttFrame::Partial { text, .. })) if text == "hello world"
        ));
        let final_frames = mapper
            .map(
                &session_id,
                r#"{"type":"speech","active":false,"timestamp":1.0}"#,
            )
            .unwrap();
        assert!(matches!(
            &final_frames[1].frame,
            Frame::Stt(SttFrame::Final { text, .. }) if text == "hello world"
        ));
    }

    #[test]
    fn ears_pause_and_status_are_not_turn_authoritative() {
        assert!(ears_frames(r#"{"type":"pause","timestamp":1.0}"#).is_empty());
        assert!(
            ears_frames(r#"{"type":"status","paused":false,"vad":true,"timestamps":false}"#)
                .is_empty()
        );
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
