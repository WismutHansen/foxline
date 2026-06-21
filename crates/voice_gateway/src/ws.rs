use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::{self, Duration};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

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
    protocol::{ClientControl, ServerEvent},
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
                        let resolved = match resolver.resolve(&workspace, loadout.as_deref()) {
                            Ok(resolved) => resolved,
                            Err(err) => {
                                send_error(&mut ws, "loadout_resolution_failed", &err.to_string())
                                    .await?;
                                continue;
                            }
                        };
                        let identity = BrainIdentity::new(
                            agent.clone(),
                            std::path::PathBuf::from(&workspace),
                            resolved.name.clone(),
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
                            },
                        )
                        .await?;
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
                    ClientControl::Interrupt => {
                        trace.event(EVENT_BARGE_IN_RECEIVED, json!({}))?;
                        if let Some(brain) = &brain {
                            brain.lock().await.abort().await?;
                        }
                        if let Some(tts_adapter) = tts.as_deref_mut() {
                            let cancel = tts_adapter.cancel(session_id.clone()).await?;
                            handle_runtime_frame(
                                &mut ws,
                                &mut pipeline,
                                &trace,
                                &avatar_router,
                                &mut live,
                                brain.as_ref(),
                                &mut tts,
                                cancel,
                            ).await?;
                        }
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Turn(TurnFrame::Interrupted {
                                reason: InterruptReason::FrontendBargeIn,
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
            if !matches!(ch, '.' | '!' | '?' | ';' | ':') {
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
                        let request = tts.speak(frame.session_id.clone(), sentence).await?;
                        process_pipeline_outputs(ws, pipeline, trace, avatar_router, request)
                            .await?;
                    }
                }
            }
            Frame::Brain(BrainFrame::Done) => {
                if let (Some(tts), Some(text)) = (tts.as_deref_mut(), live.sentence_buffer.flush())
                {
                    let request = tts.speak(frame.session_id.clone(), text).await?;
                    process_pipeline_outputs(ws, pipeline, trace, avatar_router, request).await?;
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
    crate::tts::ensure_supported_tts_backend(name)?;
    let repo = repo_root();
    let worker = env::var("CODEC_TTS_WORKER_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo.join("services/qwen3_tts_worker.py"));
    let reference = resolve_voice_reference(persona, workspace, &repo)?;
    let sample_rate = env::var("CODEC_TTS_WORKER_SAMPLE_RATE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(24_000);
    let mut args = vec![
        "run".to_string(),
        "--no-project".to_string(),
        "--with".to_string(),
        "speech-to-speech==0.2.9".to_string(),
        "python".to_string(),
        worker.display().to_string(),
        "--serve".to_string(),
        "--model-name".to_string(),
        env::var("CODEC_TTS_MODEL")
            .or_else(|_| env::var("QWEN3_TTS_MODEL"))
            .unwrap_or_else(|_| "mlx-community/Qwen3-TTS-12Hz-0.6B-Base-4bit".to_string()),
        "--ref-audio".to_string(),
        reference.wav.display().to_string(),
        "--ref-text-file".to_string(),
        reference.txt.display().to_string(),
        "--language".to_string(),
        env::var("QWEN3_TTS_LANGUAGE").unwrap_or_else(|_| "auto".to_string()),
        "--output-sample-rate".to_string(),
        sample_rate.to_string(),
        "--temperature".to_string(),
        env::var("QWEN3_TTS_TEMPERATURE").unwrap_or_else(|_| "0.7".to_string()),
        "--top-k".to_string(),
        env::var("QWEN3_TTS_TOP_K").unwrap_or_else(|_| "30".to_string()),
        "--blocksize".to_string(),
        env::var("CODEC_TTS_WORKER_BLOCKSIZE").unwrap_or_else(|_| "2048".to_string()),
    ];
    if let Ok(seed) = env::var("QWEN3_TTS_SEED") {
        args.push("--seed".to_string());
        args.push(seed);
    }
    let mut config = QwenWorkerConfig::new(
        env::var("CODEC_TTS_WORKER_COMMAND").unwrap_or_else(|_| "uv".to_string()),
        args,
        repo,
    );
    config.output_sample_rate_hz = sample_rate;
    Ok(Box::new(QwenWorkerTtsAdapter::new(config)))
}

struct VoiceReference {
    wav: PathBuf,
    txt: PathBuf,
}

fn resolve_voice_reference(
    persona: &str,
    workspace: &std::path::Path,
    repo: &std::path::Path,
) -> Result<VoiceReference> {
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

    use tempfile::tempdir;

    use super::{resolve_voice_reference, SentenceBuffer};

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
    fn voice_reference_resolves_repo_persona_sidecars() {
        let repo = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let ref_dir = repo.path().join("personas/campbell/voice/reference_audio");
        fs::create_dir_all(&ref_dir).unwrap();
        fs::write(ref_dir.join("voice.wav"), b"wav").unwrap();
        fs::write(ref_dir.join("voice.txt"), "reference transcript").unwrap();

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
