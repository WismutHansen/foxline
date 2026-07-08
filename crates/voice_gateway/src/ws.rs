use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{self, Duration};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

use foxline_protocol::{ClientControl, ServerEvent};

use crate::{
    avatar::AvatarActionRouter,
    brain::{BrainIdentity, BrainPool, PiRpcBrain},
    config::GatewayConfig,
    frame::{
        AudioFrame, BrainFrame, Frame, FrameEnvelope, FrontendToolFrame, InterruptReason,
        LifecycleFrame, SessionId, SttFrame, TtsFrame, TurnFrame, VadFrame,
    },
    loadout::LoadoutResolver,
    pipeline::{default_pipeline, LinearPipeline},
    stt::{stt_trace_event, ParakeetSileroConfig, ParakeetSileroSttAdapter, SttAdapter},
    tools::{FrontendToolNegotiation, FrontendToolRouter},
    trace::{
        TraceWriter, EVENT_BARGE_IN_RECEIVED, EVENT_BRAIN_FIRST_TOKEN, EVENT_BRAIN_REQUEST_START,
        EVENT_FRONTEND_AUDIO_PLAY_SCHEDULED, EVENT_MIC_FRAME_RECEIVED, EVENT_TTS_CANCEL_SENT,
    },
    tts::{qwen_worker_trace_event, QwenWorkerConfig, QwenWorkerTtsAdapter, TtsAdapter},
};

pub async fn serve(config: GatewayConfig) -> Result<()> {
    let listener = TcpListener::bind(&config.bind)
        .await
        .with_context(|| format!("bind gateway websocket {}", config.bind))?;
    info!(bind = %config.bind, "voice gateway listening");
    let brain_pool = Arc::new(BrainPool::new(config.brain.clone()));
    let shared = Arc::new(config);

    loop {
        let (stream, addr) = listener.accept().await.context("accept websocket tcp")?;
        let config = Arc::clone(&shared);
        let brain_pool = Arc::clone(&brain_pool);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, config, brain_pool).await {
                error!(%addr, error = %err, "gateway connection failed");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    config: Arc<GatewayConfig>,
    brain_pool: Arc<BrainPool>,
) -> Result<()> {
    let mut ws = accept_async(stream).await.context("accept websocket")?;
    let session_id = SessionId::new();
    let session_id_text = session_id.0.to_string();
    let trace = TraceWriter::create(config.trace_dir.clone(), &session_id_text)?;
    let mut capabilities_declared = false;
    let mut frontend_capability_profile = json!({});
    let mut advertised_frontend_tools = Vec::<String>::new();
    let mut input_sample_rate_hz = 16_000;
    let mut avatar_router = AvatarActionRouter::default();
    let mut tool_router: Option<FrontendToolRouter> = None;
    let mut brain: Option<Arc<Mutex<PiRpcBrain>>> = None;
    let mut stt: Option<Box<dyn SttAdapter>> = None;
    let mut tts: Option<Box<dyn TtsAdapter>> = None;
    let mut live = LiveSessionState::new(session_id_text.clone());
    let mut session_started = false;
    let mut pipeline = default_pipeline(config.turn.clone());
    let mut adapter_tick = time::interval(Duration::from_millis(15));

    send_event(
        &mut ws,
        &ServerEvent::Hello {
            protocol_version: 1,
            binary_audio: true,
        },
    )
    .await?;

    loop {
        tokio::select! {
            _ = adapter_tick.tick(), if session_started => {
                drain_adapter_frames(
                    &mut ws,
                    &mut pipeline,
                    &trace,
                    &avatar_router,
                    &mut live,
                    brain.as_ref(),
                    &mut stt,
                    &mut tts,
                ).await?;
                continue;
            }
            message = ws.next() => {
                let Some(message) = message else { break; };
                match message.context("read websocket message")? {
            Message::Text(text) => {
                let event = match serde_json::from_str::<ClientControl>(&text) {
                    Ok(event) => event,
                    Err(err) => {
                        send_error(&mut ws, "invalid_json", &err.to_string()).await?;
                        continue;
                    }
                };
                match event {
                    ClientControl::Hello { capabilities, .. } => {
                        capabilities_declared = true;
                        advertised_frontend_tools = capabilities.tools.clone();
                        if let Some(audio) = &capabilities.audio {
                            if let Some(sample_rate_hz) = audio.sample_rates_hz.first() {
                                input_sample_rate_hz = *sample_rate_hz;
                            }
                        }
                        avatar_router =
                            AvatarActionRouter::new(capabilities.avatar_actions.clone());
                        frontend_capability_profile = serde_json::to_value(&capabilities)?;
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Lifecycle(LifecycleFrame::CapabilityDeclared {
                                profile: frontend_capability_profile.clone(),
                            }),
                        );
                        let _ = pipeline.process(frame).await?;
                    }
                    ClientControl::StartSession {
                        agent,
                        workspace,
                        loadout,
                        persona,
                        model,
                    } => {
                        let persona = persona.unwrap_or_else(|| agent.clone());
                        if config.frontend.require_capability_declaration && !capabilities_declared
                        {
                            send_error(
                                &mut ws,
                                "capabilities_required",
                                "send hello before start_session",
                            )
                            .await?;
                            continue;
                        }
                        let resolver = LoadoutResolver::new(config.loadouts.clone());
                        let mut resolved = match resolver.resolve(&workspace, loadout.as_deref()) {
                            Ok(resolved) => resolved,
                            Err(err) => {
                                send_error(&mut ws, "loadout_resolution_failed", &err.to_string())
                                    .await?;
                                continue;
                            }
                        };
                        let selected_model = model
                            .as_ref()
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                            .or_else(|| resolved.loadout.pi.model.clone());
                        resolved.loadout.pi.model = selected_model.clone();
                        let identity = BrainIdentity::new(
                            agent.clone(),
                            std::path::PathBuf::from(&workspace),
                            resolved.name.clone(),
                            selected_model.clone(),
                            &frontend_capability_profile,
                        );
                        let tool_negotiation = FrontendToolNegotiation::negotiate(
                            &resolved.loadout.tools,
                            &advertised_frontend_tools,
                        );
                        if let Err(err) = tool_negotiation.ensure_startup_allowed() {
                            send_error(&mut ws, "frontend_tools_missing", &err.to_string()).await?;
                            continue;
                        }
                        let negotiated_tools = tool_negotiation
                            .effective
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>();
                        tool_router = Some(FrontendToolRouter::new(tool_negotiation.clone()));
                        let brain_handle =
                            match brain_pool.get_or_prewarm(identity.clone(), &resolved).await {
                                Ok(brain) => brain,
                                Err(err) => {
                                    send_error(&mut ws, "brain_prewarm_failed", &err.to_string())
                                        .await?;
                                    continue;
                                }
                            };
                        let launch = brain_handle.lock().await.launch().clone();
                        let stt_adapter = match build_stt_adapter(&resolved.loadout.adapters.stt) {
                            Ok(adapter) => adapter,
                            Err(err) => {
                                send_error(&mut ws, "stt_adapter_failed", &err.to_string()).await?;
                                continue;
                            }
                        };
                        let mut tts_adapter = match build_tts_adapter(
                            &resolved.loadout.adapters.tts,
                            &persona,
                            &resolved.workspace,
                        ) {
                            Ok(adapter) => adapter,
                            Err(err) => {
                                send_error(&mut ws, "tts_adapter_failed", &err.to_string()).await?;
                                continue;
                            }
                        };
                        if let Err(err) = tts_adapter.prewarm().await {
                            send_error(&mut ws, "tts_prewarm_failed", &err.to_string()).await?;
                            continue;
                        }
                        trace.event(
                            "loadout_resolved",
                            json!({
                                "agent": agent,
                                "persona": persona.clone(),
                                "workspace": workspace,
                                "loadout": resolved.name,
                                "source": resolved.source.as_ref().map(|path| path.display().to_string()),
                                "extensions": resolved.extension_paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                                "stt": resolved.loadout.adapters.stt,
                                "tts": resolved.loadout.adapters.tts,
                                "prewarm": resolved.loadout.lifecycle.prewarm,
                                "keep_warm_ms": resolved.loadout.lifecycle.keep_warm_ms,
                                "frontend_tools": negotiated_tools,
                            }),
                        )?;
                        trace.event(
                            "brain_identity_bound",
                            json!({
                                "agent": identity.agent,
                                "workspace": identity.workspace.display().to_string(),
                                "loadout": identity.loadout,
                                "frontend_capability_hash": identity.frontend_capability_hash,
                                "pi_command": launch.command,
                                "pi_args": launch.args,
                            }),
                        )?;
                        session_started = true;
                        brain = Some(brain_handle);
                        stt = Some(stt_adapter);
                        tts = Some(tts_adapter);
                        live = LiveSessionState::new(session_id_text.clone());
                        live.persona = Some(persona);
                        process_pipeline_outputs(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            tool_negotiation.negotiated_frame(session_id.clone()),
                        )
                        .await?;
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Lifecycle(LifecycleFrame::SessionStarted),
                        );
                        process_pipeline_outputs(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            frame,
                        )
                        .await?;
                        send_event(
                            &mut ws,
                            &ServerEvent::SessionStarted {
                                session_id: session_id_text.clone(),
                                model: selected_model.clone(),
                            },
                        )
                        .await?;
                        if let Some(brain) = brain.as_ref() {
                            brain.lock().await.get_available_models().await?;
                        }
                    }
                    ClientControl::EndSession => {
                        if let Some(stt) = stt.as_deref_mut() {
                            let _ = stt.shutdown().await;
                        }
                        if let Some(tts) = tts.as_deref_mut() {
                            let _ = tts.shutdown().await;
                        }
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Lifecycle(LifecycleFrame::SessionEnded),
                        );
                        process_pipeline_outputs(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            frame,
                        )
                        .await?;
                        send_event(&mut ws, &ServerEvent::SessionEnded).await?;
                        break;
                    }
                    ClientControl::VadHint {
                        speaking,
                        confidence,
                    } => {
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Vad(VadFrame::FrontendHint {
                                speaking,
                                confidence,
                            }),
                        );
                        process_pipeline_outputs(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            frame,
                        )
                        .await?;
                    }
                    ClientControl::TextUtterance { text } => {
                        if !session_started {
                            send_error(
                                &mut ws,
                                "session_required",
                                "start_session before text_utterance",
                            )
                            .await?;
                            continue;
                        }
                        let text = text.trim().to_string();
                        if text.is_empty() {
                            send_error(&mut ws, "empty_text_utterance", "text_utterance text is empty").await?;
                            continue;
                        }
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Stt(SttFrame::Final {
                                text: text.clone(),
                                confidence: Some(1.0),
                            }),
                        );
                        handle_runtime_frame(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            &mut live,
                            brain.as_ref(),
                            &mut tts,
                            frame,
                        )
                        .await?;
                    }
                    ClientControl::SwitchModel { model } => {
                        let model = model.trim().to_string();
                        if model.is_empty() {
                            send_error(&mut ws, "empty_model", "switch_model model is empty").await?;
                            continue;
                        }
                        let Some(brain) = brain.as_ref() else {
                            send_error(&mut ws, "session_required", "start_session before switch_model").await?;
                            continue;
                        };
                        trace.event("model_switch_requested", json!({ "model": model }))?;
                        brain.lock().await.set_model(&model).await?;
                    }
                    ClientControl::Interrupt => {
                        trace.event(EVENT_BARGE_IN_RECEIVED, json!({}))?;
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Turn(TurnFrame::Interrupted {
                                reason: InterruptReason::FrontendBargeIn,
                            }),
                        );
                        handle_runtime_frame(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            &mut live,
                            brain.as_ref(),
                            &mut tts,
                            frame,
                        )
                        .await?;
                    }
                    ClientControl::FrontendToolResult { call_id, result } => {
                        if let Some(router) = &tool_router {
                            let frame = router.result_frame(session_id.clone(), call_id, result);
                            process_pipeline_outputs(
                                &mut ws,
                                &mut pipeline,
                                &trace,
                                &avatar_router,
                                frame,
                            )
                            .await?;
                        } else {
                            warn!("frontend tool result received before session negotiation");
                            send_error(
                                &mut ws,
                                "frontend_tools_not_negotiated",
                                "start a session before sending frontend tool results",
                            )
                            .await?;
                        }
                    }
                }
            }
            Message::Binary(bytes) => {
                if !session_started {
                    send_error(
                        &mut ws,
                        "session_required",
                        "start_session before binary audio",
                    )
                    .await?;
                    continue;
                }
                if bytes.len() > config.frontend.max_audio_frame_bytes {
                    send_error(
                        &mut ws,
                        "audio_frame_too_large",
                        "binary audio frame exceeds configured limit",
                    )
                    .await?;
                    continue;
                }
                trace.event(
                    EVENT_MIC_FRAME_RECEIVED,
                    json!({ "bytes": bytes.len(), "sample_rate_hz": input_sample_rate_hz }),
                )?;
                let frame = FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Audio(AudioFrame::InputPcm {
                        sample_rate_hz: input_sample_rate_hz,
                        channels: 1,
                        bytes: bytes.into(),
                    }),
                );
                if let Frame::Audio(AudioFrame::InputPcm { bytes, .. }) = &frame.frame {
                    if let Some(stt) = stt.as_deref_mut() {
                        let stt_hint = stt.send_pcm(session_id.clone(), bytes.clone()).await?;
                        handle_runtime_frame(
                            &mut ws,
                            &mut pipeline,
                            &trace,
                            &avatar_router,
                            &mut live,
                            brain.as_ref(),
                            &mut tts,
                            stt_hint,
                        ).await?;
                    }
                }
                handle_runtime_frame(
                    &mut ws,
                    &mut pipeline,
                    &trace,
                    &avatar_router,
                    &mut live,
                    brain.as_ref(),
                    &mut tts,
                    frame,
                ).await?;
            }
            Message::Close(_) => break,
            Message::Ping(payload) => ws.send(Message::Pong(payload)).await?,
            Message::Pong(_) => {}
            Message::Frame(_) => {}
        }
            }
        }
    }

    info!(session_id = %session_id_text, "gateway session ended");
    Ok(())
}

struct LiveSessionState {
    turn_id: String,
    persona: Option<String>,
    assistant_started: bool,
    saw_brain_first_token: bool,
    sentence_buffer: SentenceBuffer,
}

impl LiveSessionState {
    fn new(turn_id: String) -> Self {
        Self {
            turn_id,
            persona: None,
            assistant_started: false,
            saw_brain_first_token: false,
            sentence_buffer: SentenceBuffer::default(),
        }
    }

    fn reset_assistant(&mut self) {
        self.assistant_started = false;
        self.saw_brain_first_token = false;
        self.sentence_buffer.clear();
    }
}

#[derive(Default)]
struct SentenceBuffer {
    text: String,
}

impl SentenceBuffer {
    fn push(&mut self, delta: &str) -> Vec<String> {
        self.text.push_str(delta);
        let mut out = Vec::new();
        while let Some(cut) = self.find_cut() {
            let sentence = self.text[..cut].trim().to_string();
            self.text = self.text[cut..].trim_start().to_string();
            if !sentence.is_empty() {
                out.push(sentence);
            }
        }
        out
    }

    fn flush(&mut self) -> Option<String> {
        let text = self.text.trim().to_string();
        self.text.clear();
        (!text.is_empty()).then_some(text)
    }

    fn clear(&mut self) {
        self.text.clear();
    }

    fn find_cut(&self) -> Option<usize> {
        const MIN_CHARS: usize = 28;
        const MAX_CHARS: usize = 90;
        for (idx, ch) in self.text.char_indices() {
            if !is_likely_tts_boundary(&self.text, idx, ch) {
                continue;
            }
            let cut = idx + ch.len_utf8();
            if cut >= MIN_CHARS {
                return Some(cut);
            }
        }
        (self.text.len() > MAX_CHARS).then_some(MAX_CHARS)
    }
}

fn is_likely_tts_boundary(text: &str, idx: usize, ch: char) -> bool {
    if !matches!(ch, '.' | '!' | '?' | ';' | ':') {
        return false;
    }
    let prev = text[..idx].chars().next_back().unwrap_or_default();
    let next = text[idx + ch.len_utf8()..]
        .chars()
        .next()
        .unwrap_or_default();
    if ch == '.' && prev.is_ascii_digit() && next.is_ascii_digit() {
        return false;
    }
    if ch == '.' {
        let before = text[..idx]
            .split_whitespace()
            .next_back()
            .unwrap_or_default()
            .trim_matches(|c: char| !c.is_ascii_alphabetic() && c != '.');
        let normalized = before.trim_end_matches('.').to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "mr" | "mrs" | "ms" | "dr" | "prof" | "sr" | "jr" | "st" | "vs" | "etc" | "e.g" | "i.e"
        ) {
            return false;
        }
        if before.len() == 1 && before.chars().all(|c| c.is_ascii_uppercase()) {
            return false;
        }
    }
    true
}

fn model_label_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(model) = value.get("model") {
        return model_label_from_value(model);
    }
    let provider = value.get("provider").and_then(|value| value.as_str());
    let id = value
        .get("id")
        .or_else(|| value.get("modelId"))
        .or_else(|| value.get("model_id"))
        .and_then(|value| value.as_str());
    match (provider, id) {
        (Some(provider), Some(id)) if !provider.is_empty() => Some(format!("{provider}/{id}")),
        (_, Some(id)) => Some(id.to_string()),
        _ => value.as_str().map(ToString::to_string),
    }
}

fn model_labels_from_available_models(value: &serde_json::Value) -> Vec<String> {
    value
        .get("models")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(model_label_from_value)
        .collect()
}

fn normalize_tts_text(text: &str) -> Option<String> {
    let without_code = remove_fenced_blocks(text);
    let mut joined = String::new();
    let mut in_table = false;
    for raw_line in without_code.lines() {
        let table_line = markdown_table_row_to_speech(raw_line, in_table);
        if table_line.is_some() {
            in_table = true;
        } else if !raw_line.trim().is_empty() {
            in_table = false;
        }
        let line = table_line.unwrap_or_else(|| strip_markdown_line(raw_line));
        let line = line.trim();
        if !line.is_empty() {
            push_tts_segment(&mut joined, line);
        }
    }
    let joined = collapse_whitespace(&joined);
    (!joined.is_empty()).then_some(joined)
}

fn push_tts_segment(out: &mut String, segment: &str) {
    if !out.is_empty() {
        if !out.ends_with(['.', '!', '?', ';', ':']) {
            out.push('.');
        }
        out.push(' ');
    }
    out.push_str(segment);
}

fn markdown_table_row_to_speech(line: &str, in_table: bool) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return None;
    }
    if trimmed
        .chars()
        .all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
    {
        return Some(String::new());
    }
    if !in_table {
        return Some(String::new());
    }
    let cells = trimmed
        .trim_matches('|')
        .split('|')
        .map(strip_markdown_line)
        .map(|cell| cell.trim().to_string())
        .filter(|cell| !cell.is_empty())
        .collect::<Vec<_>>();
    if cells.is_empty() || cells.iter().all(|cell| cell.chars().all(|ch| ch == '-')) {
        return Some(String::new());
    }
    Some(cells.join(": "))
}

fn strip_markdown_line(line: &str) -> String {
    let mut out = strip_links_and_images(line);
    out = out.trim_start().to_string();
    while out.starts_with('#') {
        out.remove(0);
    }
    out = out.trim_start().to_string();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = out.strip_prefix(marker) {
            out = rest.to_string();
            break;
        }
    }
    let digit_prefix_len = out
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    if digit_prefix_len > 0 {
        let rest = &out[digit_prefix_len..];
        if let Some(after_marker) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            out = after_marker.trim_start().to_string();
        }
    }
    out.replace(['*', '_', '~', '`', '|'], " ")
        .replace(['[', ']', '(', ')'], " ")
}

fn strip_links_and_images(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let image = rest[..start].ends_with('!');
        if image {
            out.pop();
        }
        let after_start = &rest[start + 1..];
        let Some(end_label) = after_start.find(']') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let label = &after_start[..end_label];
        let after_label = &after_start[end_label + 1..];
        if let Some(after_url) = after_label.strip_prefix('(') {
            if let Some(end_url) = after_url.find(')') {
                if !image {
                    out.push_str(label);
                }
                rest = &after_url[end_url + 1..];
                continue;
            }
        }
        out.push_str(&rest[start..start + 1 + end_label + 1]);
        rest = after_label;
    }
    out.push_str(rest);
    out
}

fn remove_fenced_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if !in_fence {
                out.push_str(" code block omitted. ");
            }
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn drain_adapter_frames(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    pipeline: &mut LinearPipeline,
    trace: &TraceWriter,
    avatar_router: &AvatarActionRouter,
    live: &mut LiveSessionState,
    brain: Option<&Arc<Mutex<PiRpcBrain>>>,
    stt: &mut Option<Box<dyn SttAdapter>>,
    tts: &mut Option<Box<dyn TtsAdapter>>,
) -> Result<()> {
    if let Some(stt) = stt.as_deref_mut() {
        while let Some(frame) = stt.try_next_frame() {
            handle_runtime_frame(ws, pipeline, trace, avatar_router, live, brain, tts, frame)
                .await?;
        }
    }
    if let Some(brain) = brain {
        loop {
            let frame = { brain.lock().await.try_next_frame() };
            let Some(frame) = frame else { break };
            handle_runtime_frame(
                ws,
                pipeline,
                trace,
                avatar_router,
                live,
                Some(brain),
                tts,
                frame,
            )
            .await?;
        }
    }
    loop {
        let frame = tts.as_deref_mut().and_then(TtsAdapter::try_next_frame);
        let Some(frame) = frame else { break };
        let mut no_tts = None;
        handle_runtime_frame(
            ws,
            pipeline,
            trace,
            avatar_router,
            live,
            brain,
            &mut no_tts,
            frame,
        )
        .await?;
    }
    Ok(())
}

async fn handle_runtime_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    pipeline: &mut LinearPipeline,
    trace: &TraceWriter,
    avatar_router: &AvatarActionRouter,
    live: &mut LiveSessionState,
    brain: Option<&Arc<Mutex<PiRpcBrain>>>,
    tts: &mut Option<Box<dyn TtsAdapter>>,
    frame: FrameEnvelope,
) -> Result<()> {
    if let Some((event, data)) = stt_trace_event(&frame) {
        trace.event(event, data)?;
    }
    if let Some((event, data)) = qwen_worker_trace_event(&frame) {
        trace.event(event, data)?;
    }
    let frames = process_pipeline_outputs(ws, pipeline, trace, avatar_router, frame).await?;
    for frame in frames {
        match frame.frame {
            Frame::Stt(SttFrame::Partial { text, confidence }) => {
                send_event(
                    ws,
                    &ServerEvent::UserTranscript {
                        text,
                        final_: false,
                        confidence,
                    },
                )
                .await?;
            }
            Frame::Stt(SttFrame::Final { text, confidence }) => {
                send_event(
                    ws,
                    &ServerEvent::UserTranscript {
                        text,
                        final_: true,
                        confidence,
                    },
                )
                .await?;
            }
            Frame::Stt(SttFrame::Error { message }) => {
                send_error(ws, "stt_error", &message).await?;
            }
            Frame::Turn(TurnFrame::UserStarted) => {
                send_event(
                    ws,
                    &ServerEvent::Phase {
                        phase: "listening".to_string(),
                    },
                )
                .await?;
            }
            Frame::Turn(TurnFrame::UserCommitted { text }) => {
                if let Some(brain) = brain {
                    let request = brain.lock().await.prompt(&text).await?;
                    trace.event(
                        EVENT_BRAIN_REQUEST_START,
                        json!({ "text_chars": text.chars().count() }),
                    )?;
                    process_pipeline_outputs(ws, pipeline, trace, avatar_router, request).await?;
                    send_event(
                        ws,
                        &ServerEvent::Phase {
                            phase: "thinking".to_string(),
                        },
                    )
                    .await?;
                }
            }
            Frame::Turn(TurnFrame::Interrupted { .. }) => {
                if let Some(brain) = brain {
                    brain.lock().await.abort().await?;
                    trace.event("brain_abort_sent", json!({}))?;
                }
                if let Some(tts_adapter) = tts.as_deref_mut() {
                    let cancel = tts_adapter.cancel(frame.session_id.clone()).await?;
                    process_pipeline_outputs(ws, pipeline, trace, avatar_router, cancel).await?;
                }
                live.reset_assistant();
                send_event(
                    ws,
                    &ServerEvent::AudioReset {
                        reason: Some("interrupted".to_string()),
                    },
                )
                .await?;
                send_event(
                    ws,
                    &ServerEvent::Phase {
                        phase: "interrupted".to_string(),
                    },
                )
                .await?;
            }
            Frame::Brain(BrainFrame::TextDelta { text }) => {
                if !live.assistant_started {
                    live.assistant_started = true;
                    send_event(
                        ws,
                        &ServerEvent::TurnStarted {
                            turn_id: live.turn_id.clone(),
                            character: live.persona.clone(),
                            persona: live.persona.clone(),
                        },
                    )
                    .await?;
                }
                if !live.saw_brain_first_token {
                    live.saw_brain_first_token = true;
                    trace.event(EVENT_BRAIN_FIRST_TOKEN, json!({}))?;
                }
                send_event(
                    ws,
                    &ServerEvent::AssistantDelta {
                        turn_id: live.turn_id.clone(),
                        delta: text.clone(),
                    },
                )
                .await?;
                if let Some(tts) = tts.as_deref_mut() {
                    for sentence in live.sentence_buffer.push(&text) {
                        if let Some(tts_text) = normalize_tts_text(&sentence) {
                            let request = tts.speak(frame.session_id.clone(), tts_text).await?;
                            process_pipeline_outputs(ws, pipeline, trace, avatar_router, request)
                                .await?;
                        }
                    }
                }
            }
            Frame::Brain(BrainFrame::Done) => {
                if let (Some(tts), Some(text)) = (tts.as_deref_mut(), live.sentence_buffer.flush())
                {
                    if let Some(tts_text) = normalize_tts_text(&text) {
                        let request = tts.speak(frame.session_id.clone(), tts_text).await?;
                        process_pipeline_outputs(ws, pipeline, trace, avatar_router, request)
                            .await?;
                    }
                }
                send_event(
                    ws,
                    &ServerEvent::TurnCompleted {
                        turn_id: live.turn_id.clone(),
                    },
                )
                .await?;
                send_event(
                    ws,
                    &ServerEvent::Phase {
                        phase: "speaking".to_string(),
                    },
                )
                .await?;
            }
            Frame::Brain(BrainFrame::Error { message }) => {
                send_error(ws, "brain_error", &message).await?;
            }
            Frame::Brain(BrainFrame::ToolCall { name, arguments }) => {
                send_event(ws, &ServerEvent::FrontendToolCall { name, arguments }).await?;
            }
            Frame::Tts(TtsFrame::Error { message }) => {
                send_error(ws, "tts_error", &message).await?;
            }
            Frame::Metrics(metrics) if metrics.event == "pi_model_changed" => {
                if let Some(model) = model_label_from_value(&metrics.data) {
                    send_event(ws, &ServerEvent::ModelChanged { model }).await?;
                }
            }
            Frame::Metrics(metrics) if metrics.event == "pi_available_models" => {
                send_event(
                    ws,
                    &ServerEvent::ModelsAvailable {
                        models: model_labels_from_available_models(&metrics.data),
                        current: None,
                    },
                )
                .await?;
            }
            Frame::Audio(AudioFrame::OutputPcm { bytes, .. }) => {
                trace.event(
                    EVENT_FRONTEND_AUDIO_PLAY_SCHEDULED,
                    json!({ "bytes": bytes.len() }),
                )?;
                ws.send(Message::Binary(bytes.to_vec())).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn process_pipeline_outputs(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    pipeline: &mut LinearPipeline,
    trace: &TraceWriter,
    avatar_router: &AvatarActionRouter,
    frame: FrameEnvelope,
) -> Result<Vec<FrameEnvelope>> {
    let out = pipeline.process(frame).await?;
    for frame in &out {
        match &frame.frame {
            Frame::Turn(TurnFrame::UserStarted) => {
                trace.event("turn_user_started", json!({}))?;
            }
            Frame::Turn(TurnFrame::UserCommitted { text }) => {
                trace.event(
                    "turn_user_committed",
                    json!({ "chars": text.chars().count() }),
                )?;
            }
            Frame::Turn(TurnFrame::Interrupted { reason }) => {
                trace.event("turn_interrupted", json!({ "reason": reason }))?;
            }
            Frame::Tts(crate::frame::TtsFrame::Cancel) => {
                trace.event(EVENT_TTS_CANCEL_SENT, json!({}))?;
            }
            Frame::AvatarAction(action) => {
                if let Some(action) = avatar_router.route(action) {
                    send_event(ws, &ServerEvent::AvatarAction { action }).await?;
                } else {
                    trace.event("avatar_action_rejected", json!({ "reason": "unsupported" }))?;
                }
            }
            Frame::FrontendTool(FrontendToolFrame::Negotiated { tools }) => {
                send_event(
                    ws,
                    &ServerEvent::FrontendToolsNegotiated {
                        tools: tools.clone(),
                    },
                )
                .await?;
                trace.event("frontend_tools_negotiated", json!({ "tools": tools }))?;
            }
            Frame::FrontendTool(FrontendToolFrame::Call { name, arguments }) => {
                send_event(
                    ws,
                    &ServerEvent::FrontendToolCall {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                )
                .await?;
            }
            Frame::FrontendTool(FrontendToolFrame::Rejected { name, reason }) => {
                send_event(
                    ws,
                    &ServerEvent::FrontendToolRejected {
                        name: name.clone(),
                        reason: reason.clone(),
                    },
                )
                .await?;
                trace.event(
                    "frontend_tool_rejected",
                    json!({ "name": name, "reason": reason }),
                )?;
            }
            Frame::FrontendTool(FrontendToolFrame::Result { call_id, .. }) => {
                trace.event("frontend_tool_result", json!({ "call_id": call_id }))?;
            }
            _ => {}
        }
    }
    Ok(out)
}

fn build_stt_adapter(name: &str) -> Result<Box<dyn SttAdapter>> {
    crate::stt::ensure_supported_stt_backend(name)?;
    let config = ParakeetSileroConfig {
        url: env::var("FOXLINE_STT_WS_URL")
            .or_else(|_| env::var("VITE_PARAKEET_CPP_STT_URL"))
            .unwrap_or_else(|_| "ws://127.0.0.1:8796/ws".to_string()),
        ..ParakeetSileroConfig::default()
    };
    Ok(Box::new(ParakeetSileroSttAdapter::new(config)))
}

fn build_tts_adapter(
    name: &str,
    persona: &str,
    workspace: &std::path::Path,
) -> Result<Box<dyn TtsAdapter>> {
    // FOXLINE_TTS_BACKEND overrides the loadout's adapter name so a backend
    // can be A/B'd without editing loadouts.
    let backend = env::var("FOXLINE_TTS_BACKEND").unwrap_or_else(|_| name.to_string());
    crate::tts::ensure_supported_tts_backend(&backend)?;
    let repo = repo_root();
    let reference = resolve_voice_reference(persona, workspace, &repo)?;
    let sample_rate = env::var("CODEC_TTS_WORKER_SAMPLE_RATE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(24_000);
    let model = env::var("CODEC_TTS_MODEL")
        .or_else(|_| env::var("QWEN3_TTS_MODEL"))
        .unwrap_or_else(|_| "mlx-community/Qwen3-TTS-12Hz-0.6B-Base-6bit".to_string());

    // Both workers speak the same binary frame protocol and (deliberately)
    // the same CLI; only the launch command and model reference differ.
    let (command, mut args) = match backend.as_str() {
        "rust-mlx" => {
            let binary = resolve_rust_tts_worker(&repo)?;
            // The native worker loads from a local directory, not an HF id.
            let model_path = env::var("FOXLINE_TTS_MODEL_PATH")
                .map(PathBuf::from)
                .ok()
                .or_else(|| hf_snapshot_path(&model))
                .with_context(|| {
                    format!(
                        "rust-mlx TTS backend needs a local model directory: set \
                         FOXLINE_TTS_MODEL_PATH or download {model} into the \
                         Hugging Face cache first"
                    )
                })?;
            (
                binary.display().to_string(),
                vec![
                    "--serve".to_string(),
                    "--model-path".to_string(),
                    model_path.display().to_string(),
                ],
            )
        }
        _ => {
            let worker = env::var("CODEC_TTS_WORKER_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| repo.join("services/qwen3_tts_worker.py"));
            (
                env::var("CODEC_TTS_WORKER_COMMAND").unwrap_or_else(|_| "uv".to_string()),
                vec![
                    "run".to_string(),
                    "--no-project".to_string(),
                    "--with".to_string(),
                    "speech-to-speech==0.2.9".to_string(),
                    "python".to_string(),
                    worker.display().to_string(),
                    "--serve".to_string(),
                    "--model-name".to_string(),
                    model,
                ],
            )
        }
    };

    args.extend([
        "--ref-audio".to_string(),
        reference.wav.display().to_string(),
        "--ref-text-file".to_string(),
        reference.txt.display().to_string(),
        "--language".to_string(),
        env::var("QWEN3_TTS_LANGUAGE").unwrap_or_else(|_| "auto".to_string()),
        "--output-sample-rate".to_string(),
        sample_rate.to_string(),
        "--temperature".to_string(),
        env::var("QWEN3_TTS_TEMPERATURE").unwrap_or_else(|_| "0.9".to_string()),
        "--top-k".to_string(),
        env::var("QWEN3_TTS_TOP_K").unwrap_or_else(|_| "50".to_string()),
        "--blocksize".to_string(),
        env::var("CODEC_TTS_WORKER_BLOCKSIZE").unwrap_or_else(|_| "2048".to_string()),
    ]);
    if let Ok(seed) = env::var("QWEN3_TTS_SEED") {
        args.push("--seed".to_string());
        args.push(seed);
    }
    let mut config = QwenWorkerConfig::new(command, args, repo);
    config.output_sample_rate_hz = sample_rate;
    Ok(Box::new(QwenWorkerTtsAdapter::new(config)))
}

/// Locate the native spqx TTS worker: explicit env, then the sibling spqx
/// checkout's release build, then PATH.
fn resolve_rust_tts_worker(repo: &Path) -> Result<PathBuf> {
    if let Ok(path) = env::var("FOXLINE_TTS_RUST_WORKER") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!(
            "FOXLINE_TTS_RUST_WORKER points at {}, which does not exist",
            path.display()
        );
    }
    if let Some(parent) = repo.parent() {
        let sibling = parent.join("spqx/target/release/pibot-tts-worker");
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    if let Ok(output) = std::process::Command::new("which")
        .arg("pibot-tts-worker")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    anyhow::bail!(
        "rust-mlx TTS backend selected but no worker binary found: set \
         FOXLINE_TTS_RUST_WORKER, build ../spqx (cargo build --release \
         --no-default-features --features mlx --bin pibot-tts-worker), or put \
         pibot-tts-worker on PATH"
    )
}

/// Resolve a Hugging Face model id to its local cache snapshot directory.
fn hf_snapshot_path(model_id: &str) -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let cache = env::var("HF_HOME")
        .map(|hf| PathBuf::from(hf).join("hub"))
        .unwrap_or_else(|_| PathBuf::from(&home).join(".cache/huggingface/hub"));
    let repo_dir = cache.join(format!("models--{}", model_id.replace('/', "--")));
    let snapshots = repo_dir.join("snapshots");
    let mut entries: Vec<_> = std::fs::read_dir(&snapshots)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    entries.pop()
}

struct VoiceReference {
    wav: PathBuf,
    txt: PathBuf,
}

#[derive(Debug, Deserialize)]
struct PersonaManifest {
    voice: Option<PersonaVoiceManifest>,
}

#[derive(Debug, Deserialize)]
struct PersonaVoiceManifest {
    reference_audio: Option<String>,
    reference_text: Option<String>,
}

fn resolve_voice_reference(persona: &str, workspace: &Path, repo: &Path) -> Result<VoiceReference> {
    if let (Ok(wav), Ok(txt)) = (
        env::var("FOXLINE_TTS_REF_AUDIO").or_else(|_| env::var("CODEC_TTS_REF_AUDIO")),
        env::var("FOXLINE_TTS_REF_TEXT_FILE").or_else(|_| env::var("CODEC_TTS_REF_TEXT_FILE")),
    ) {
        return Ok(VoiceReference {
            wav: PathBuf::from(wav),
            txt: PathBuf::from(txt),
        });
    }
    let candidates = [
        workspace.join(".foxline").join("personas").join(persona),
        workspace.join("personas").join(persona),
        workspace.join("agents").join(persona),
        repo.join("personas").join(persona),
        repo.join("agents").join(persona),
    ];
    for character_dir in candidates {
        if let Some(reference) = resolve_manifest_voice_reference(&character_dir) {
            return Ok(reference);
        }
        for dir in [
            character_dir.join("voice/reference_audio"),
            character_dir.join("assets/reference_audio"),
            character_dir.join("assets"),
        ] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                {
                    let txt = path
                        .with_extension("txt")
                        .exists()
                        .then(|| path.with_extension("txt"))
                        .or_else(|| {
                            let shared = dir.join("reference.txt");
                            shared.exists().then_some(shared)
                        })
                        .or_else(|| {
                            let sidecar = PathBuf::from(format!("{}.txt", path.display()));
                            sidecar.exists().then_some(sidecar)
                        });
                    if let Some(txt) = txt {
                        return Ok(VoiceReference { wav: path, txt });
                    }
                }
            }
        }
    }
    anyhow::bail!(
        "No TTS reference wav/transcript found for persona {persona}; set FOXLINE_TTS_REF_AUDIO and FOXLINE_TTS_REF_TEXT_FILE"
    )
}

fn resolve_manifest_voice_reference(persona_dir: &Path) -> Option<VoiceReference> {
    let manifest_path = persona_dir.join("persona.toml");
    let manifest = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest: PersonaManifest = toml::from_str(&manifest).ok()?;
    let voice = manifest.voice?;
    let wav = voice.reference_audio?;
    let wav = persona_dir.join(wav);
    if !wav.exists() {
        return None;
    }
    let txt = voice
        .reference_text
        .map(|path| persona_dir.join(path))
        .filter(|path| path.exists())
        .or_else(|| {
            let path = wav.with_extension("txt");
            path.exists().then_some(path)
        })
        .or_else(|| {
            let path = PathBuf::from(format!("{}.txt", wav.display()));
            path.exists().then_some(path)
        })?;
    Some(VoiceReference { wav, txt })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

async fn send_error<S>(ws: &mut S, code: &str, message: &str) -> Result<()>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    send_event(
        ws,
        &ServerEvent::Error {
            code: code.to_string(),
            message: message.to_string(),
        },
    )
    .await
}

async fn send_event<S>(ws: &mut S, event: &ServerEvent) -> Result<()>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    ws.send(Message::Text(serde_json::to_string(event)?))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        model_label_from_value, model_labels_from_available_models, normalize_tts_text,
        resolve_voice_reference, SentenceBuffer,
    };

    #[test]
    fn model_label_helpers_format_pi_model_responses() {
        let data = json!({
            "models": [
                { "provider": "anthropic", "id": "claude-sonnet-4" },
                { "provider": "LM-Studio", "id": "gemma-4-26b-a4b-it" }
            ]
        });
        assert_eq!(
            model_labels_from_available_models(&data),
            vec![
                "anthropic/claude-sonnet-4".to_string(),
                "LM-Studio/gemma-4-26b-a4b-it".to_string(),
            ]
        );
        assert_eq!(
            model_label_from_value(&json!({ "provider": "openai", "modelId": "gpt-4o" }))
                .as_deref(),
            Some("openai/gpt-4o")
        );
    }

    #[test]
    fn assistant_delta_sent_to_frontend_matches_text_sent_to_tts() {
        let pi_text = "Loud and clear, Snake. I'm right here. What's your status?";
        let mut tts = SentenceBuffer::default();
        let mut frontend_text = String::new();
        let mut tts_text = Vec::new();

        for delta in [
            "Loud and clear, Snake. I",
            "'m right here. What's your status?",
        ] {
            frontend_text.push_str(delta);
            tts_text.extend(tts.push(delta));
        }
        if let Some(tail) = tts.flush() {
            tts_text.push(tail);
        }

        assert_eq!(frontend_text, pi_text);
        assert_eq!(tts_text.join(" "), pi_text);
    }

    #[test]
    fn sentence_buffer_emits_sentence_sized_chunks_and_flushes_tail() {
        let mut buffer = SentenceBuffer::default();

        assert!(buffer.push("Short.").is_empty());
        let chunks = buffer.push(" This is long enough to speak now.");

        assert_eq!(chunks, vec!["Short. This is long enough to speak now."]);
        assert!(buffer.push(" Tail without punctuation").is_empty());
        assert_eq!(buffer.flush().as_deref(), Some("Tail without punctuation"));
    }

    #[test]
    fn sentence_buffer_preserves_decimal_boundaries_for_tts() {
        let mut buffer = SentenceBuffer::default();

        let chunks = buffer.push("Tune to 140.85 and wait for confirmation.");

        assert_eq!(chunks, vec!["Tune to 140.85 and wait for confirmation."]);
    }

    #[test]
    fn tts_normalizer_converts_markdown_tables_to_speakable_text() {
        let text = r#"
## Today

| Time | Event | Location |
|------|-------|----------|
| **08:00-09:00** | WG: Green.OWL | Teams |
| **11:00-11:40** | KI-Keynote | IoT-Center |
"#;

        assert_eq!(
            normalize_tts_text(text).as_deref(),
            Some("Today. 08:00-09:00: WG: Green.OWL: Teams. 11:00-11:40: KI-Keynote: IoT-Center")
        );
    }

    #[test]
    fn tts_normalizer_strips_markdown_noise_without_stripping_plain_numbers() {
        let text = r#"
1. **NVIDIA NGC** | [API paths](https://example.com)
- 200-Festangestellte bleiben relevant.
```json
{"debug": true}
```
![chart](chart.png)
"#;

        assert_eq!(
            normalize_tts_text(text).as_deref(),
            Some("NVIDIA NGC API paths. 200-Festangestellte bleiben relevant. code block omitted.")
        );
    }

    #[test]
    fn voice_reference_resolves_repo_persona_sidecars() {
        let repo = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let ref_dir = repo.path().join("personas/campbell/voice/reference_audio");
        let persona_dir = repo.path().join("personas/campbell");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("voice.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("voice.txt"), "reference transcript").unwrap();
        fs::write(
            persona_dir.join("persona.toml"),
            r#"
[voice]
reference_audio = "voice/reference_audio/voice.wav"
reference_text = "voice/reference_audio/voice.txt"
"#,
        )
        .unwrap();

        let reference = resolve_voice_reference("campbell", workspace.path(), repo.path()).unwrap();

        assert_eq!(reference.wav, ref_dir.join("voice.wav"));
        assert_eq!(reference.txt, ref_dir.join("voice.txt"));
    }

    #[test]
    fn voice_reference_resolves_workspace_persona_sidecars() {
        let repo = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let ref_dir = workspace
            .path()
            .join(".foxline/personas/radio-operator/voice/reference_audio");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("operator.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("operator.wav.txt"), "reference transcript").unwrap();

        let reference =
            resolve_voice_reference("radio-operator", workspace.path(), repo.path()).unwrap();

        assert_eq!(reference.wav, ref_dir.join("operator.wav"));
        assert_eq!(reference.txt, ref_dir.join("operator.wav.txt"));
    }

    #[test]
    fn voice_reference_uses_manifest_default_before_directory_order() {
        let repo = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let persona_dir = repo.path().join("personas/mira-chen");
        let ref_dir = persona_dir.join("voice/reference_audio");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("mira-chen-a_nova-alice.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("mira-chen-c_sarah-isabella.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("reference.txt"), "reference transcript").unwrap();
        fs::write(
            persona_dir.join("persona.toml"),
            r#"
[voice]
reference_audio = "voice/reference_audio/mira-chen-c_sarah-isabella.wav"
reference_text = "voice/reference_audio/reference.txt"
"#,
        )
        .unwrap();

        let reference =
            resolve_voice_reference("mira-chen", workspace.path(), repo.path()).unwrap();

        assert_eq!(
            reference.wav,
            ref_dir.join("mira-chen-c_sarah-isabella.wav")
        );
        assert_eq!(reference.txt, ref_dir.join("reference.txt"));
    }

    #[test]
    fn voice_reference_keeps_legacy_repo_agent_fallback() {
        let repo = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let ref_dir = repo.path().join("agents/campbell/assets/reference_audio");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("voice.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("voice.txt"), "reference transcript").unwrap();

        let reference = resolve_voice_reference("campbell", workspace.path(), repo.path()).unwrap();

        assert_eq!(reference.wav, ref_dir.join("voice.wav"));
        assert_eq!(reference.txt, ref_dir.join("voice.txt"));
    }
}
