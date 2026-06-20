use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;

pub const EVENT_MIC_FRAME_RECEIVED: &str = "mic_frame_received";
pub const EVENT_STT_PARTIAL: &str = "stt_partial";
pub const EVENT_STT_FINAL: &str = "stt_final";
pub const EVENT_BRAIN_REQUEST_START: &str = "brain_request_start";
pub const EVENT_BRAIN_FIRST_TOKEN: &str = "brain_first_token";
pub const EVENT_TTS_REQUEST_START: &str = "tts_request_start";
pub const EVENT_TTS_AUDIO_START: &str = "tts_audio_start";
pub const EVENT_FRONTEND_AUDIO_PLAY_SCHEDULED: &str = "frontend_audio_play_scheduled";
pub const EVENT_BARGE_IN_RECEIVED: &str = "barge_in_received";
pub const EVENT_TTS_CANCEL_SENT: &str = "tts_cancel_sent";

pub struct TraceWriter {
    file: Mutex<File>,
}

impl TraceWriter {
    pub fn create(trace_dir: PathBuf, session_id: &str) -> Result<Self> {
        fs::create_dir_all(&trace_dir)
            .with_context(|| format!("create trace dir {}", trace_dir.display()))?;
        let path = trace_dir.join(format!("{session_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open trace file {}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn event(&self, event: impl Into<String>, data: Value) -> Result<()> {
        let record = TraceRecord {
            ts: OffsetDateTime::now_utc(),
            t_ms: current_millis(),
            event: event.into(),
            data,
        };
        let line = serde_json::to_string(&record).context("serialize trace record")?;
        let mut file = self.file.lock().expect("trace writer lock poisoned");
        writeln!(file, "{line}").context("write trace record")?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct TraceRecord {
    #[serde(with = "time::serde::rfc3339")]
    ts: OffsetDateTime,
    t_ms: u128,
    event: String,
    data: Value,
}

fn current_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
