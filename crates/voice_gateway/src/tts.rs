use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::Arc};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
};

use crate::{
    frame::{AudioFrame, Frame, FrameEnvelope, SessionId, TtsFrame},
    trace::{TraceWriter, EVENT_TTS_AUDIO_START, EVENT_TTS_CANCEL_SENT, EVENT_TTS_REQUEST_START},
};

pub const WORKER_INPUT_SPEAK: u8 = 1;
pub const WORKER_INPUT_CANCEL: u8 = 2;
pub const WORKER_INPUT_SHUTDOWN: u8 = 3;
pub const WORKER_OUTPUT_READY: u8 = 1;
pub const WORKER_OUTPUT_AUDIO_START: u8 = 2;
pub const WORKER_OUTPUT_AUDIO_CHUNK: u8 = 3;
pub const WORKER_OUTPUT_AUDIO_DONE: u8 = 4;
pub const WORKER_OUTPUT_ERROR: u8 = 5;
pub const FRAME_HEADER_BYTES: usize = 9;

#[async_trait]
pub trait TtsAdapter: Send {
    async fn prewarm(&mut self) -> Result<()>;
    async fn speak(&mut self, session_id: SessionId, text: String) -> Result<FrameEnvelope>;
    async fn cancel(&mut self, session_id: SessionId) -> Result<FrameEnvelope>;
    async fn shutdown(&mut self) -> Result<()>;
    fn try_next_frame(&mut self) -> Option<FrameEnvelope>;
    async fn next_frame(&mut self) -> Option<FrameEnvelope>;
}

#[derive(Debug, Clone)]
pub struct QwenWorkerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub output_sample_rate_hz: u32,
}

impl QwenWorkerConfig {
    pub fn new(command: impl Into<String>, args: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args,
            cwd: cwd.into(),
            output_sample_rate_hz: 24_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFrame {
    pub frame_type: u8,
    pub request_id: u32,
    pub payload: Bytes,
}

impl WorkerFrame {
    pub fn encode(frame_type: u8, request_id: u32, payload: &[u8]) -> Bytes {
        let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
        out.push(frame_type);
        out.extend_from_slice(&request_id.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        Bytes::from(out)
    }

    pub fn decode(buffer: &mut Vec<u8>) -> Result<Option<Self>> {
        if buffer.len() < FRAME_HEADER_BYTES {
            return Ok(None);
        }
        let frame_type = buffer[0];
        let request_id = u32::from_le_bytes(buffer[1..5].try_into().expect("slice length"));
        let payload_len =
            u32::from_le_bytes(buffer[5..9].try_into().expect("slice length")) as usize;
        let frame_len = FRAME_HEADER_BYTES + payload_len;
        if buffer.len() < frame_len {
            return Ok(None);
        }
        let payload = Bytes::copy_from_slice(&buffer[FRAME_HEADER_BYTES..frame_len]);
        buffer.drain(..frame_len);
        Ok(Some(Self {
            frame_type,
            request_id,
            payload,
        }))
    }
}

pub struct QwenWorkerTtsAdapter {
    config: QwenWorkerConfig,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    // Keep request generation attached across the async queue. Active requests
    // may already be removed by AudioDone before cancellation reaches us.
    events: Option<mpsc::Receiver<(u64, FrameEnvelope)>>,
    generation: u64,
    next_request_id: u32,
    active: Arc<Mutex<HashMap<u32, ActiveTtsRequest>>>,
    trace: Option<TraceWriter>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveTtsRequest {
    generation: u64,
    session_id: SessionId,
    sample_rate_hz: u32,
}

impl QwenWorkerTtsAdapter {
    pub fn new(config: QwenWorkerConfig) -> Self {
        Self {
            config,
            child: None,
            stdin: None,
            events: None,
            generation: 0,
            next_request_id: 1,
            active: Arc::new(Mutex::new(HashMap::new())),
            trace: None,
        }
    }

    pub fn with_trace(mut self, trace: TraceWriter) -> Self {
        self.trace = Some(trace);
        self
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        let mut child = Command::new(&self.config.command)
            .args(&self.config.args)
            .current_dir(&self.config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "launch Qwen TTS worker {} {:?}",
                    self.config.command, self.config.args
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .context("Qwen worker stdin unavailable")?;
        let mut stdout = child
            .stdout
            .take()
            .context("Qwen worker stdout unavailable")?;
        let (tx, rx) = mpsc::channel(128);
        let active = Arc::clone(&self.active);

        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 8192];
            loop {
                let read = match stdout.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                buffer.extend_from_slice(&chunk[..read]);
                loop {
                    match WorkerFrame::decode(&mut buffer) {
                        Ok(Some(frame)) if frame.frame_type == WORKER_OUTPUT_READY => {}
                        Ok(Some(frame)) => {
                            let request = {
                                let mut active = active.lock().await;
                                let request = active.get(&frame.request_id).cloned();
                                if matches!(
                                    frame.frame_type,
                                    WORKER_OUTPUT_AUDIO_DONE | WORKER_OUTPUT_ERROR
                                ) {
                                    active.remove(&frame.request_id);
                                }
                                request
                            };
                            let Some(request) = request else {
                                continue;
                            };
                            for out in qwen_output_to_frames(&request.session_id, &request, &frame)
                            {
                                if tx.send((request.generation, out)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => return,
                    }
                }
            }
        });

        self.stdin = Some(stdin);
        self.events = Some(rx);
        self.child = Some(child);
        Ok(())
    }

    async fn send_worker_frame(
        &mut self,
        frame_type: u8,
        request_id: u32,
        payload: &[u8],
    ) -> Result<()> {
        self.start().await?;
        let stdin = self
            .stdin
            .as_mut()
            .context("Qwen worker stdin not started")?;
        stdin
            .write_all(&WorkerFrame::encode(frame_type, request_id, payload))
            .await?;
        stdin.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl TtsAdapter for QwenWorkerTtsAdapter {
    async fn prewarm(&mut self) -> Result<()> {
        self.start().await
    }

    async fn speak(&mut self, session_id: SessionId, text: String) -> Result<FrameEnvelope> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.active.lock().await.insert(
            request_id,
            ActiveTtsRequest {
                generation: self.generation,
                session_id: session_id.clone(),
                sample_rate_hz: self.config.output_sample_rate_hz,
            },
        );
        if let Some(trace) = &self.trace {
            trace.event(
                EVENT_TTS_REQUEST_START,
                json!({ "request_id": request_id, "text_chars": text.chars().count() }),
            )?;
        }
        self.send_worker_frame(WORKER_INPUT_SPEAK, request_id, text.as_bytes())
            .await?;
        Ok(FrameEnvelope::new(
            session_id,
            Frame::Tts(TtsFrame::RequestStart { text }),
        ))
    }

    async fn cancel(&mut self, session_id: SessionId) -> Result<FrameEnvelope> {
        // Fence before any I/O: the producer may already hold a cloned request,
        // or Done may have removed it while its audio remains in the queue.
        self.generation = self.generation.wrapping_add(1);
        let ids: Vec<u32> = self.active.lock().await.drain().map(|(id, _)| id).collect();
        for request_id in ids {
            self.send_worker_frame(WORKER_INPUT_CANCEL, request_id, &[])
                .await?;
        }
        if let Some(trace) = &self.trace {
            trace.event(EVENT_TTS_CANCEL_SENT, json!({}))?;
        }
        Ok(FrameEnvelope::new(session_id, Frame::Tts(TtsFrame::Cancel)))
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.generation = self.generation.wrapping_add(1);
        self.active.lock().await.clear();
        if self.stdin.is_some() {
            let _ = self.send_worker_frame(WORKER_INPUT_SHUTDOWN, 0, &[]).await;
        }
        self.stdin = None;
        self.events = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }

    fn try_next_frame(&mut self) -> Option<FrameEnvelope> {
        let events = self.events.as_mut()?;
        while let Ok((generation, frame)) = events.try_recv() {
            if generation == self.generation {
                return Some(frame);
            }
        }
        None
    }

    async fn next_frame(&mut self) -> Option<FrameEnvelope> {
        let events = self.events.as_mut()?;
        while let Some((generation, frame)) = events.recv().await {
            if generation == self.generation {
                return Some(frame);
            }
        }
        None
    }
}

pub(crate) fn qwen_output_to_frames(
    session_id: &SessionId,
    request: &ActiveTtsRequest,
    frame: &WorkerFrame,
) -> Vec<FrameEnvelope> {
    match frame.frame_type {
        WORKER_OUTPUT_AUDIO_START => {
            let sample_rate_hz = if frame.payload.len() >= 4 {
                u32::from_le_bytes(frame.payload[..4].try_into().expect("slice length"))
            } else {
                request.sample_rate_hz
            };
            vec![FrameEnvelope::new(
                session_id.clone(),
                Frame::Tts(TtsFrame::AudioStart {
                    sample_rate_hz,
                    channels: 1,
                }),
            )]
        }
        WORKER_OUTPUT_AUDIO_CHUNK => vec![
            FrameEnvelope::new(
                session_id.clone(),
                Frame::Tts(TtsFrame::AudioChunk {
                    bytes: frame.payload.clone(),
                }),
            ),
            FrameEnvelope::new(
                session_id.clone(),
                Frame::Audio(AudioFrame::OutputPcm {
                    sample_rate_hz: request.sample_rate_hz,
                    channels: 1,
                    bytes: frame.payload.clone(),
                }),
            ),
        ],
        WORKER_OUTPUT_AUDIO_DONE => vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Tts(TtsFrame::AudioDone),
        )],
        WORKER_OUTPUT_ERROR => vec![FrameEnvelope::new(
            session_id.clone(),
            Frame::Tts(TtsFrame::Error {
                message: String::from_utf8_lossy(&frame.payload).to_string(),
            }),
        )],
        WORKER_OUTPUT_READY => Vec::new(),
        other => {
            vec![FrameEnvelope::new(
                session_id.clone(),
                Frame::Tts(TtsFrame::Error {
                    message: format!("unknown Qwen worker output frame type {other}"),
                }),
            )]
        }
    }
}

pub fn qwen_worker_trace_event(frame: &FrameEnvelope) -> Option<(&'static str, serde_json::Value)> {
    match &frame.frame {
        Frame::Tts(TtsFrame::RequestStart { text }) => Some((
            EVENT_TTS_REQUEST_START,
            json!({ "text_chars": text.chars().count() }),
        )),
        Frame::Tts(TtsFrame::AudioStart {
            sample_rate_hz,
            channels,
        }) => Some((
            EVENT_TTS_AUDIO_START,
            json!({ "sample_rate_hz": sample_rate_hz, "channels": channels }),
        )),
        Frame::Tts(TtsFrame::Cancel) => Some((EVENT_TTS_CANCEL_SENT, json!({}))),
        _ => None,
    }
}

pub fn ensure_supported_tts_backend(name: &str) -> Result<()> {
    match name {
        // qwen3-worker: python MLX worker (services/qwen3_tts_worker.py).
        // rust-mlx: native spqx worker (pibot-tts-worker), byte-identical
        // binary protocol; see ws.rs build_tts_adapter for launch resolution.
        "qwen3-worker" | "rust-mlx" => Ok(()),
        "rust-candle" | "cpp-ggml" | "elevenlabs" | "openai" => {
            bail!("TTS backend {name} is reserved but not implemented yet")
        }
        other => bail!("unsupported TTS backend {other}"),
    }
}

#[cfg(test)]
#[path = "tts_generation_tests.rs"]
mod generation_tests;

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::frame::{AudioFrame, Frame, SessionId, TtsFrame};

    use super::{
        ensure_supported_tts_backend, qwen_output_to_frames, ActiveTtsRequest, WorkerFrame,
        WORKER_OUTPUT_AUDIO_CHUNK, WORKER_OUTPUT_AUDIO_DONE, WORKER_OUTPUT_AUDIO_START,
        WORKER_OUTPUT_ERROR,
    };

    #[test]
    fn worker_frame_round_trips_with_partial_buffer() {
        let encoded = WorkerFrame::encode(3, 42, b"abc");
        let mut buffer = encoded[..5].to_vec();
        assert!(WorkerFrame::decode(&mut buffer).unwrap().is_none());
        buffer.extend_from_slice(&encoded[5..]);

        let decoded = WorkerFrame::decode(&mut buffer).unwrap().unwrap();

        assert_eq!(decoded.frame_type, 3);
        assert_eq!(decoded.request_id, 42);
        assert_eq!(decoded.payload, Bytes::from_static(b"abc"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn qwen_audio_start_maps_to_tts_audio_start() {
        let session_id = SessionId::new();
        let request = request(&session_id);
        let frame = WorkerFrame {
            frame_type: WORKER_OUTPUT_AUDIO_START,
            request_id: 1,
            payload: Bytes::from(24_000_u32.to_le_bytes().to_vec()),
        };

        let out = qwen_output_to_frames(&session_id, &request, &frame);

        assert!(matches!(
            out[0].frame,
            Frame::Tts(TtsFrame::AudioStart {
                sample_rate_hz: 24_000,
                channels: 1
            })
        ));
    }

    #[test]
    fn qwen_audio_chunk_maps_to_tts_and_binary_audio_frames() {
        let session_id = SessionId::new();
        let request = request(&session_id);
        let frame = WorkerFrame {
            frame_type: WORKER_OUTPUT_AUDIO_CHUNK,
            request_id: 1,
            payload: Bytes::from_static(b"pcm"),
        };

        let out = qwen_output_to_frames(&session_id, &request, &frame);

        assert!(matches!(
            out[0].frame,
            Frame::Tts(TtsFrame::AudioChunk { .. })
        ));
        assert!(matches!(
            out[1].frame,
            Frame::Audio(AudioFrame::OutputPcm {
                sample_rate_hz: 24_000,
                channels: 1,
                ..
            })
        ));
    }

    #[test]
    fn qwen_done_and_error_map_to_tts_frames() {
        let session_id = SessionId::new();
        let request = request(&session_id);
        let done = WorkerFrame {
            frame_type: WORKER_OUTPUT_AUDIO_DONE,
            request_id: 1,
            payload: Bytes::new(),
        };
        let error = WorkerFrame {
            frame_type: WORKER_OUTPUT_ERROR,
            request_id: 1,
            payload: Bytes::from_static(b"failed"),
        };

        assert!(matches!(
            qwen_output_to_frames(&session_id, &request, &done)[0].frame,
            Frame::Tts(TtsFrame::AudioDone)
        ));
        assert!(matches!(
            &qwen_output_to_frames(&session_id, &request, &error)[0].frame,
            Frame::Tts(TtsFrame::Error { message }) if message == "failed"
        ));
    }

    #[test]
    fn backend_selection_names_are_explicit() {
        assert!(ensure_supported_tts_backend("qwen3-worker").is_ok());
        assert!(ensure_supported_tts_backend("openai").is_err());
        assert!(ensure_supported_tts_backend("unknown").is_err());
    }

    fn request(session_id: &SessionId) -> ActiveTtsRequest {
        ActiveTtsRequest {
            generation: 0,
            session_id: session_id.clone(),
            sample_rate_hz: 24_000,
        }
    }
}
