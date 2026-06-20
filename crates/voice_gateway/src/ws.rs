use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::{
    avatar::AvatarActionRouter,
    brain::{BrainIdentity, BrainPool},
    config::GatewayConfig,
    frame::{
        AudioFrame, Frame, FrameEnvelope, InterruptReason, LifecycleFrame, SessionId, TurnFrame,
        VadFrame,
    },
    loadout::LoadoutResolver,
    pipeline::{default_pipeline, LinearPipeline},
    protocol::{ClientControl, ServerEvent},
    trace::{
        TraceWriter, EVENT_BARGE_IN_RECEIVED, EVENT_MIC_FRAME_RECEIVED, EVENT_TTS_CANCEL_SENT,
    },
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
    let mut avatar_router = AvatarActionRouter::default();
    let mut session_started = false;
    let mut pipeline = default_pipeline(config.turn.clone());

    send_event(
        &mut ws,
        &ServerEvent::Hello {
            protocol_version: 1,
            binary_audio: true,
        },
    )
    .await?;

    while let Some(message) = ws.next().await {
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
                    } => {
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
                        let brain =
                            match brain_pool.get_or_prewarm(identity.clone(), &resolved).await {
                                Ok(brain) => brain,
                                Err(err) => {
                                    send_error(&mut ws, "brain_prewarm_failed", &err.to_string())
                                        .await?;
                                    continue;
                                }
                            };
                        let launch = brain.lock().await.launch().clone();
                        trace.event(
                            "loadout_resolved",
                            json!({
                                "agent": agent,
                                "workspace": workspace,
                                "loadout": resolved.name,
                                "source": resolved.source.as_ref().map(|path| path.display().to_string()),
                                "extensions": resolved.extension_paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                                "stt": resolved.loadout.adapters.stt,
                                "tts": resolved.loadout.adapters.tts,
                                "prewarm": resolved.loadout.lifecycle.prewarm,
                                "keep_warm_ms": resolved.loadout.lifecycle.keep_warm_ms,
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
                    ClientControl::FrontendToolResult { .. } => {
                        warn!("frontend tool result received before tool router is implemented");
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
                trace.event(EVENT_MIC_FRAME_RECEIVED, json!({ "bytes": bytes.len() }))?;
                let frame = FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Audio(AudioFrame::InputPcm {
                        sample_rate_hz: 16_000,
                        channels: 1,
                        bytes: bytes.into(),
                    }),
                );
                process_pipeline_outputs(&mut ws, &mut pipeline, &trace, &avatar_router, frame)
                    .await?;
            }
            Message::Close(_) => break,
            Message::Ping(payload) => ws.send(Message::Pong(payload)).await?,
            Message::Pong(_) => {}
            Message::Frame(_) => {}
        }
    }

    info!(session_id = %session_id_text, "gateway session ended");
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
            _ => {}
        }
    }
    Ok(out)
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
