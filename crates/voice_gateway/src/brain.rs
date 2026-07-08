use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    config::BrainConfig,
    frame::{BrainFrame, Frame, FrameEnvelope, MetricsFrame, SessionId},
    loadout::ResolvedLoadout,
};

const VOICE_SESSION_APPEND_SYSTEM_PROMPT: &str = r#"You are currently connected through a real-time voice interface. Output must be directly speakable aloud.

Voice output rules:
- Do not output Markdown tables, pipe tables, code fences, headings, horizontal rules, block quotes, or decorative separators.
- Prefer short spoken sentences and compact lists in plain prose.
- For calendar, email, search, or tabular data, summarize the most important entries in words instead of formatting a table.
- Do not include emojis, bullets made from symbols, raw URLs, markup syntax, or bracketed UI labels unless the user explicitly asks for exact text.
- Never reveal hidden reasoning, thought traces, channel markers, tool protocol text, or implementation details. If a tool fails, state the exact visible error plainly."#;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BrainIdentity {
    pub agent: String,
    pub workspace: PathBuf,
    pub loadout: String,
    pub model: Option<String>,
    pub frontend_capability_hash: String,
}

impl BrainIdentity {
    pub fn new(
        agent: impl Into<String>,
        workspace: impl Into<PathBuf>,
        loadout: impl Into<String>,
        model: Option<String>,
        frontend_capability_profile: &Value,
    ) -> Self {
        Self {
            agent: agent.into(),
            workspace: workspace.into(),
            loadout: loadout.into(),
            model,
            frontend_capability_hash: stable_value_hash(frontend_capability_profile),
        }
    }

    pub fn session_name(&self) -> String {
        format!(
            "foxline-voice-{}-{}-{}",
            sanitize_identity_component(&self.agent),
            sanitize_identity_component(&self.loadout),
            &self.frontend_capability_hash[..12]
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl PiLaunch {
    pub fn build(
        config: &BrainConfig,
        identity: &BrainIdentity,
        loadout: &ResolvedLoadout,
    ) -> Self {
        let mut args = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "--continue".to_string(),
        ];

        if config.no_context_files {
            args.push("--no-context-files".to_string());
        }
        if config.no_extensions {
            args.push("--no-extensions".to_string());
        }
        if let Some(session_dir) = &loadout.loadout.pi.session_dir {
            args.push("--session-dir".to_string());
            args.push(
                resolve_workspace_path(&loadout.workspace, session_dir)
                    .display()
                    .to_string(),
            );
        }
        if let Some(profile) = &loadout.loadout.pi.profile {
            args.push("--profile".to_string());
            args.push(profile.clone());
        }
        if let Some(pi_config) = &loadout.loadout.pi.config {
            args.push("--config".to_string());
            args.push(
                resolve_workspace_path(&loadout.workspace, pi_config)
                    .display()
                    .to_string(),
            );
        }
        if let Some(model) = &loadout.loadout.pi.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        if let Some(thinking) = &loadout.loadout.pi.thinking {
            args.push("--thinking".to_string());
            args.push(thinking.clone());
        }
        if !loadout.loadout.tools.allowed_pi.is_empty() {
            args.push("--tools".to_string());
            args.push(loadout.loadout.tools.allowed_pi.join(","));
        }
        if let Some(prompt) = &loadout.loadout.pi.append_system_prompt {
            if !prompt.trim().is_empty() {
                args.push("--append-system-prompt".to_string());
                args.push(prompt.clone());
            }
        }
        if let Some(prompt_file) = &loadout.loadout.pi.append_system_prompt_file {
            let path = resolve_workspace_path(&loadout.workspace, prompt_file);
            if let Ok(prompt) = fs::read_to_string(&path) {
                if !prompt.trim().is_empty() {
                    args.push("--append-system-prompt".to_string());
                    args.push(prompt);
                }
            }
        }
        args.push("--append-system-prompt".to_string());
        args.push(VOICE_SESSION_APPEND_SYSTEM_PROMPT.to_string());
        for extension in &loadout.extension_paths {
            args.push("--extension".to_string());
            args.push(extension.display().to_string());
        }

        let _ = identity;
        Self {
            command: config.pi_command.clone(),
            args,
            cwd: loadout.workspace.clone(),
        }
    }
}

pub struct BrainPool {
    config: BrainConfig,
    brains: Mutex<HashMap<BrainIdentity, Arc<Mutex<PiRpcBrain>>>>,
}

impl BrainPool {
    pub fn new(config: BrainConfig) -> Self {
        Self {
            config,
            brains: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_or_prewarm(
        &self,
        identity: BrainIdentity,
        loadout: &ResolvedLoadout,
    ) -> Result<Arc<Mutex<PiRpcBrain>>> {
        let mut brains = self.brains.lock().await;
        if let Some(brain) = brains.get(&identity) {
            return Ok(Arc::clone(brain));
        }

        let launch = PiLaunch::build(&self.config, &identity, loadout);
        let mut brain = PiRpcBrain::new(identity.clone(), launch, self.config.idle_timeout_ms);
        if self.config.prewarm || loadout.loadout.lifecycle.prewarm {
            brain.start().await?;
        }
        let brain = Arc::new(Mutex::new(brain));
        brains.insert(identity, Arc::clone(&brain));
        Ok(brain)
    }

    pub async fn shutdown_idle(&self) -> Result<usize> {
        let brains = self.brains.lock().await;
        let mut stopped = 0;
        for brain in brains.values() {
            let mut brain = brain.lock().await;
            if brain.is_idle_expired() {
                brain.shutdown().await?;
                stopped += 1;
            }
        }
        Ok(stopped)
    }

    pub async fn shutdown_all(&self) -> Result<()> {
        let brains = self.brains.lock().await;
        for brain in brains.values() {
            brain.lock().await.shutdown().await?;
        }
        Ok(())
    }
}

pub struct PiRpcBrain {
    identity: BrainIdentity,
    launch: PiLaunch,
    idle_timeout: Duration,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    events: Option<mpsc::Receiver<FrameEnvelope>>,
    session_id: SessionId,
    seq: u64,
    last_used: Instant,
}

impl PiRpcBrain {
    pub fn new(identity: BrainIdentity, launch: PiLaunch, idle_timeout_ms: u64) -> Self {
        Self {
            identity,
            launch,
            idle_timeout: Duration::from_millis(idle_timeout_ms),
            child: None,
            stdin: None,
            events: None,
            session_id: SessionId::new(),
            seq: 0,
            last_used: Instant::now(),
        }
    }

    pub fn identity(&self) -> &BrainIdentity {
        &self.identity
    }

    pub fn launch(&self) -> &PiLaunch {
        &self.launch
    }

    pub fn is_running(&mut self) -> bool {
        match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn is_idle_expired(&self) -> bool {
        self.last_used.elapsed() >= self.idle_timeout
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let mut child = Command::new(&self.launch.command)
            .args(&self.launch.args)
            .current_dir(&self.launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "launch Pi RPC Brain {} {:?}",
                    self.launch.command, self.launch.args
                )
            })?;

        let stdin = child.stdin.take().context("Pi RPC stdin unavailable")?;
        let stdout = child.stdout.take().context("Pi RPC stdout unavailable")?;
        let (tx, rx) = mpsc::channel(128);
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut mapper = RpcFrameMapper::default();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match mapper.line_to_frames(&session_id, &line) {
                    Ok(frames) => {
                        for frame in frames {
                            if tx.send(frame).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(err) => {
                        let frame = FrameEnvelope::new(
                            session_id.clone(),
                            Frame::Brain(BrainFrame::Error {
                                message: format!("bad Pi RPC event: {err}"),
                            }),
                        );
                        if tx.send(frame).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        self.stdin = Some(stdin);
        self.events = Some(rx);
        self.child = Some(child);
        self.send(json!({ "type": "set_session_name", "name": self.identity.session_name() }))
            .await?;
        self.send(json!({ "type": "get_state" })).await?;
        Ok(())
    }

    pub async fn restart(&mut self) -> Result<()> {
        self.shutdown().await?;
        self.start().await
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.stdin = None;
        self.events = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }

    pub async fn prompt(&mut self, text: &str) -> Result<FrameEnvelope> {
        self.start().await?;
        self.last_used = Instant::now();
        self.send(json!({
            "type": "prompt",
            "message": text,
            "streamingBehavior": "followUp",
        }))
        .await?;
        Ok(FrameEnvelope::new(
            self.session_id.clone(),
            Frame::Brain(BrainFrame::RequestStart {
                text: text.to_string(),
            }),
        ))
    }

    pub async fn abort(&mut self) -> Result<()> {
        self.start().await?;
        self.last_used = Instant::now();
        self.send(json!({ "type": "abort" })).await
    }

    pub async fn set_model(&mut self, model: &str) -> Result<()> {
        self.start().await?;
        self.last_used = Instant::now();
        let (provider, model_id) = parse_provider_model(model);
        self.send(json!({ "type": "set_model", "provider": provider, "modelId": model_id }))
            .await
    }

    pub async fn get_available_models(&mut self) -> Result<()> {
        self.start().await?;
        self.last_used = Instant::now();
        self.send(json!({ "type": "get_available_models" })).await
    }

    pub async fn new_session(&mut self) -> Result<()> {
        self.start().await?;
        self.last_used = Instant::now();
        self.send(json!({ "type": "new_session" })).await?;
        self.send(json!({ "type": "set_session_name", "name": self.identity.session_name() }))
            .await
    }

    pub async fn next_frame(&mut self) -> Option<FrameEnvelope> {
        self.events.as_mut()?.recv().await
    }

    pub fn try_next_frame(&mut self) -> Option<FrameEnvelope> {
        self.events.as_mut()?.try_recv().ok()
    }

    async fn send(&mut self, mut value: Value) -> Result<()> {
        self.seq += 1;
        value["id"] = Value::String(format!("req-{}", self.seq));
        let stdin = self.stdin.as_mut().context("Pi RPC stdin not started")?;
        stdin
            .write_all(serde_json::to_string(&value)?.as_bytes())
            .await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcEvent {
    #[serde(rename = "type")]
    event_type: String,
    assistant_message_event: Option<AssistantMessageEvent>,
    data: Option<Value>,
    message: Option<Value>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    command: Option<String>,
    success: Option<bool>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AssistantMessageEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<String>,
    channel: Option<String>,
    phase: Option<String>,
}

pub fn rpc_line_to_frames(session_id: &SessionId, line: &str) -> Result<Vec<FrameEnvelope>> {
    let event: RpcEvent = serde_json::from_str(line)?;
    Ok(RpcFrameMapper::default().event_to_frames(session_id, event))
}

#[derive(Default)]
struct RpcFrameMapper {
    think_filter: crate::think_block_filter::ThinkBlockFilter,
}

impl RpcFrameMapper {
    fn line_to_frames(&mut self, session_id: &SessionId, line: &str) -> Result<Vec<FrameEnvelope>> {
        let event: RpcEvent = serde_json::from_str(line)?;
        Ok(self.event_to_frames(session_id, event))
    }

    fn event_to_frames(&mut self, session_id: &SessionId, event: RpcEvent) -> Vec<FrameEnvelope> {
        let mut frames = Vec::new();
        match event.event_type.as_str() {
            "message_update" => {
                if let Some(assistant) = event.assistant_message_event {
                    // Single source of truth: visible assistant text is emitted ONLY
                    // from a text_delta event. Reasoning lives in separate
                    // thinking_* events which are ignored here by construction, and the
                    // terminal text_end event (full cumulative content) is never read
                    // for text, so reasoning can never leak and the answer can never
                    // be duplicated. Two residual provider misbehaviors are guarded:
                    // a text_delta tagged with a thinking channel/phase (metadata-only
                    // check, no text inspection) and <think>...</think> tags injected
                    // into the visible text itself, handled by ThinkBlockFilter.
                    match assistant.event_type.as_str() {
                        "text_delta" => {
                            if is_thinking_channel(&assistant) {
                                return frames;
                            }
                            if let Some(delta) = assistant.delta {
                                let delta = self.think_filter.filter(&delta);
                                if delta.trim().is_empty() {
                                    return frames;
                                }
                                frames.push(FrameEnvelope::new(
                                    session_id.clone(),
                                    Frame::Brain(BrainFrame::TextDelta { text: delta }),
                                ));
                            }
                        }
                        "done" => {
                            self.think_filter =
                                crate::think_block_filter::ThinkBlockFilter::default();
                            frames.push(FrameEnvelope::new(
                                session_id.clone(),
                                Frame::Brain(BrainFrame::Done),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            "tool_execution_start" => frames.push(FrameEnvelope::new(
                session_id.clone(),
                Frame::Brain(BrainFrame::ToolCall {
                    name: event.tool_name.unwrap_or_else(|| "unknown".to_string()),
                    arguments: event.message.or(event.data).unwrap_or_else(|| json!({})),
                }),
            )),
            "tool_execution_end" => frames.push(FrameEnvelope::new(
                session_id.clone(),
                Frame::Brain(BrainFrame::ToolResult {
                    call_id: event.tool_call_id.unwrap_or_default(),
                    result: event.data.unwrap_or_else(|| json!({})),
                }),
            )),
            "response"
                if event.command.as_deref() == Some("prompt") && event.success == Some(false) =>
            {
                frames.push(FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Brain(BrainFrame::Error {
                        message: value_to_string(event.error, "Pi RPC prompt rejected"),
                    }),
                ));
            }
            "response"
                if event.command.as_deref() == Some("set_model") && event.success == Some(true) =>
            {
                frames.push(FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Metrics(MetricsFrame {
                        event: "pi_model_changed".to_string(),
                        elapsed_ms: None,
                        data: event.data.unwrap_or_else(|| json!({})),
                    }),
                ));
            }
            "response"
                if event.command.as_deref() == Some("get_available_models")
                    && event.success == Some(true) =>
            {
                frames.push(FrameEnvelope::new(
                    session_id.clone(),
                    Frame::Metrics(MetricsFrame {
                        event: "pi_available_models".to_string(),
                        elapsed_ms: None,
                        data: event.data.unwrap_or_else(|| json!({})),
                    }),
                ));
            }
            "error" => frames.push(FrameEnvelope::new(
                session_id.clone(),
                Frame::Brain(BrainFrame::Error {
                    message: value_to_string(event.error.or(event.message), "Pi RPC error"),
                }),
            )),
            "agent_end" => frames.push(FrameEnvelope::new(
                session_id.clone(),
                Frame::Brain(BrainFrame::Done),
            )),
            _ => {}
        }
        frames
    }
}

/// Structured guard for providers that tag reasoning as a `text_delta` with a
/// thinking channel/phase instead of using `thinking_*` events. Inspects event
/// metadata only — never the text itself.
fn is_thinking_channel(event: &AssistantMessageEvent) -> bool {
    fn hidden(value: Option<&str>) -> bool {
        matches!(
            value.map(|value| value.trim().to_ascii_lowercase()),
            Some(value)
                if matches!(
                    value.as_str(),
                    "thought" | "thinking" | "analysis" | "reasoning"
                )
        )
    }
    hidden(event.channel.as_deref()) || hidden(event.phase.as_deref())
}

fn parse_provider_model(model: &str) -> (&str, &str) {
    model
        .split_once('/')
        .map(|(provider, model_id)| (provider, model_id))
        .unwrap_or(("", model))
}

fn resolve_workspace_path(workspace: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

fn stable_value_hash(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn sanitize_identity_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').chars().take(32).collect()
}

fn value_to_string(value: Option<Value>, fallback: &str) -> String {
    match value {
        Some(Value::String(value)) => value,
        Some(value) => value.to_string(),
        None => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{
        config::BrainConfig,
        frame::{BrainFrame, Frame, SessionId},
        loadout::{ResolvedLoadout, VoiceLoadout},
    };

    use super::{
        rpc_line_to_frames, BrainIdentity, PiLaunch, RpcEvent, RpcFrameMapper,
        VOICE_SESSION_APPEND_SYSTEM_PROMPT,
    };

    fn resolved_loadout(workspace: PathBuf) -> ResolvedLoadout {
        let mut loadout = VoiceLoadout::default();
        loadout.pi.profile = Some("voice".to_string());
        loadout.pi.config = Some(".pi/config.toml".to_string());
        loadout.pi.session_dir = Some(".pi/sessions".to_string());
        loadout.pi.model = Some("LM-Studio/gemma-4-26b-a4b-it".to_string());
        loadout.pi.thinking = Some("minimal".to_string());
        loadout.pi.append_system_prompt_file = Some("SYSTEM.md".to_string());
        loadout.tools.allowed_pi = vec!["read".to_string(), "write".to_string()];
        fs::write(workspace.join("SYSTEM.md"), "Stay concise.").unwrap();
        let extension = workspace.join(".foxline/extensions/frontend-tools");
        fs::create_dir_all(&extension).unwrap();
        ResolvedLoadout {
            name: "default".to_string(),
            workspace,
            source: None,
            loadout,
            extension_paths: vec![extension],
        }
    }

    use std::path::PathBuf;

    /// Replays a captured real Pi RPC stream (one JSON line per stdout line) through a
    /// single stateful mapper, exactly as the gateway reader loop does, and returns the
    /// concatenated visible assistant text.
    fn replay_visible_text(capture: &str) -> String {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();
        let mut visible = String::new();
        for line in capture.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            for frame in mapper.line_to_frames(&session_id, line).unwrap() {
                if let Frame::Brain(BrainFrame::TextDelta { text }) = frame.frame {
                    visible.push_str(&text);
                }
            }
        }
        visible
    }

    /// Replays a capture and returns the event types that produced at least one
    /// visible TextDelta frame. The leak invariant is structural: a `thinking_*`
    /// event may never be the source of a TextDelta.
    fn replay_text_delta_sources(capture: &str) -> Vec<String> {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();
        let mut sources = Vec::new();
        for line in capture.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let event: RpcEvent = serde_json::from_str(line).unwrap();
            let et = event
                .assistant_message_event
                .as_ref()
                .map(|a| a.event_type.clone())
                .unwrap_or_default();
            let frames = mapper.event_to_frames(&session_id, event);
            let emitted_visible = frames.iter().any(|f| {
                matches!(&f.frame, Frame::Brain(BrainFrame::TextDelta { .. }))
            });
            if emitted_visible {
                sources.push(et);
            }
        }
        sources
    }

    #[test]
    fn replays_real_pi_rpc_streams_without_leaks_or_duplication() {
        // Fixtures captured from live Pi RPC sessions against the campbell voice agent.
        // Each pair is (captured stream, Pi's authoritative full visible text).
        // Gemma/LM-Studio streams text only; Qwen/ZGX and Step/HP-Z8 stream a reasoning
        // block then the answer.
        let fixtures: &[(&str, &str)] = &[
            (
                include_str!("../assets/fixtures/rpc-streams/gemma-lm-studio.jsonl"),
                "I hear you, Snake. What's your status?",
            ),
            (
                include_str!("../assets/fixtures/rpc-streams/qwen-zgx.jsonl"),
                "Loud and clear, Snake. What do you have for me?",
            ),
            (
                include_str!("../assets/fixtures/rpc-streams/step-hp-z8.jsonl"),
                "Loud and clear, Snake. Go ahead.",
            ),
        ];

        for (capture, expected) in fixtures {
            let visible = replay_visible_text(capture);

            // Structural leak invariant: only text_delta events may produce visible
            // text. A thinking_* event emitting a TextDelta would be a reasoning leak,
            // and a text_end event emitting one would be a duplication. Both are
            // architecturally impossible under single-source-of-truth; this asserts it
            // against real provider streams without any phrase-matching.
            let sources = replay_text_delta_sources(capture);
            assert!(
                sources.iter().all(|s| s == "text_delta"),
                "non-text_delta event emitted visible text (leak/duplication): {sources:?}"
            );

            // The answer must not be duplicated (text_end replay guard).
            let occurrences = visible.matches(expected).count();
            assert_eq!(
                occurrences, 1,
                "expected the answer once, found {occurrences} in: {visible:?}"
            );

            // The trimmed visible text must equal Pi's authoritative answer exactly.
            assert_eq!(visible.trim(), *expected, "visible text mismatch");
        }
    }

    #[test]
    fn pi_launch_includes_rpc_mode_identity_config_tools_and_extensions() {
        let dir = tempdir().unwrap();
        let resolved = resolved_loadout(dir.path().to_path_buf());
        let identity = BrainIdentity::new(
            "campbell",
            dir.path(),
            "default",
            resolved.loadout.pi.model.clone(),
            &json!({ "tools": ["codec.display"] }),
        );

        let launch = PiLaunch::build(&BrainConfig::default(), &identity, &resolved);

        assert_eq!(launch.command, "pi");
        assert_eq!(launch.cwd, dir.path());
        assert!(launch.args.windows(2).any(|pair| pair == ["--mode", "rpc"]));
        assert!(launch.args.contains(&"--continue".to_string()));
        assert!(launch.args.contains(&"--no-extensions".to_string()));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--profile", "voice"]));
        assert!(launch
            .args
            .iter()
            .any(|arg| arg.ends_with(".pi/config.toml")));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--model", "LM-Studio/gemma-4-26b-a4b-it"]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--thinking", "minimal"]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--append-system-prompt", "Stay concise."]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--append-system-prompt", VOICE_SESSION_APPEND_SYSTEM_PROMPT]));
        assert!(launch
            .args
            .windows(2)
            .any(|pair| pair == ["--tools", "read,write"]));
        assert!(launch.args.contains(&"--extension".to_string()));
        assert!(launch
            .args
            .iter()
            .any(|arg| arg.ends_with(".foxline/extensions/frontend-tools")));
    }

    #[test]
    fn identity_is_bound_to_agent_loadout_workspace_and_frontend_profile() {
        let a = BrainIdentity::new(
            "campbell",
            "/tmp/a",
            "default",
            Some("model-a".to_string()),
            &json!({ "tools": ["a"] }),
        );
        let b = BrainIdentity::new(
            "campbell",
            "/tmp/a",
            "default",
            Some("model-a".to_string()),
            &json!({ "tools": ["b"] }),
        );
        let c = BrainIdentity::new(
            "campbell",
            "/tmp/a",
            "field",
            Some("model-a".to_string()),
            &json!({ "tools": ["a"] }),
        );
        let d = BrainIdentity::new(
            "campbell",
            "/tmp/a",
            "default",
            Some("model-b".to_string()),
            &json!({ "tools": ["a"] }),
        );

        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert!(a
            .session_name()
            .starts_with("foxline-voice-campbell-default-"));
    }

    #[test]
    fn maps_pi_rpc_text_delta_and_done_events_to_brain_frames() {
        let session_id = SessionId::new();
        let frames = rpc_line_to_frames(
            &session_id,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hello"}}"#,
        )
        .unwrap();

        assert!(matches!(
            &frames[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "hello"
        ));

        let done = rpc_line_to_frames(
            &session_id,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"done"}}"#,
        )
        .unwrap();
        assert!(matches!(done[0].frame, Frame::Brain(BrainFrame::Done)));
    }

    #[test]
    fn text_delta_content_field_is_never_read_for_visible_text() {
        // Real Pi streams carry visible text exclusively in text_delta.delta;
        // the content field (cumulative state) must never be re-emitted, or the
        // answer duplicates for display and TTS.
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let first = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Copy that."}}"#,
            )
            .unwrap();
        let with_content = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":" Stay sharp.","content":"Copy that. Stay sharp."}}"#,
            )
            .unwrap();
        let content_only = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","content":"Copy that. Stay sharp out there."}}"#,
            )
            .unwrap();

        assert!(matches!(
            &first[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "Copy that."
        ));
        assert_eq!(with_content.len(), 1);
        assert!(matches!(
            &with_content[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == " Stay sharp."
        ));
        assert!(content_only.is_empty());
    }

    #[test]
    fn thinking_block_content_never_leaks_as_visible_text() {
        // Captured from a real Pi RPC stream (zgx/qwen3.6-35b). Pi separates the
        // reasoning into thinking_* deltas and the answer into text_* deltas, and the
        // thinking_end block carries the full reasoning text in its `content` field.
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        // A reasoning chunk arrives on the thinking channel.
        let _ = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"The user is reaching out to me. I should stay in character."}}"#,
            )
            .unwrap();

        // The visible answer arrives on the text channel.
        let answer = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Loud and clear, Snake. What do you have for me?"}}"#,
            )
            .unwrap();

        // thinking_end carries the full reasoning text in `content`.
        let thinking_end = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_end","content":"The user is reaching out to me. I should stay in character."}}"#,
            )
            .unwrap();

        // text_end carries the full visible text in `content`.
        let text_end = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","content":"Loud and clear, Snake. What do you have for me?"}}"#,
            )
            .unwrap();

        let visible: Vec<String> = [&answer, &thinking_end, &text_end]
            .into_iter()
            .flatten()
            .filter_map(|frame| match &frame.frame {
                Frame::Brain(BrainFrame::TextDelta { text }) => Some(text.clone()),
                _ => None,
            })
            .collect();

        let leaked = visible
            .iter()
            .any(|text| text.contains("The user") || text.contains("stay in character"));
        assert!(!leaked, "reasoning leaked into visible text: {visible:?}");
        assert_eq!(
            visible.concat(),
            "Loud and clear, Snake. What do you have for me?"
        );
    }

    #[test]
    fn text_end_full_content_does_not_duplicate_streamed_answer() {
        // Captured shape from ZGX/Step providers: the answer streams in as small
        // text_delta chunks (delta only), then text_end arrives carrying the FULL
        // visible content. The mapper must not re-emit that full content as a second
        // delta, which would duplicate the assistant's answer for both display and TTS.
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let answer = "Loud and clear, Snake. What do you have for me?";
        for delta in [
            "\n\n",
            "L",
            "oud",
            " and",
            " clear",
            ",",
            " Snake.",
            " What do you have for me?",
        ] {
            let _ = mapper
                .line_to_frames(
                    &session_id,
                    &format!(
                        r#"{{"type":"message_update","assistantMessageEvent":{{"type":"text_delta","delta":{}}}}}"#,
                        serde_json::Value::String(delta.to_string())
                    ),
                )
                .unwrap();
        }
        let text_end = mapper
            .line_to_frames(
                &session_id,
                &format!(
                    r#"{{"type":"message_update","assistantMessageEvent":{{"type":"text_end","content":{}}}}}"#,
                    serde_json::Value::String(format!("\n\n{answer}"))
                ),
            )
            .unwrap();

        let visible: Vec<String> = text_end
            .iter()
            .filter_map(|frame| match &frame.frame {
                Frame::Brain(BrainFrame::TextDelta { text }) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(
            visible.is_empty(),
            "text_end re-emitted streamed answer as a duplicate delta: {visible:?}"
        );
    }

    #[test]
    fn maps_pi_rpc_model_responses_to_metrics_frames() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let available = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"response","command":"get_available_models","success":true,"data":{"models":[{"provider":"anthropic","id":"claude-sonnet-4"},{"provider":"LM-Studio","id":"gemma-4-26b-a4b-it"}]}}"#,
            )
            .unwrap();
        let switched = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"response","command":"set_model","success":true,"data":{"provider":"anthropic","id":"claude-sonnet-4"}}"#,
            )
            .unwrap();

        assert_eq!(available.len(), 1);
        assert!(matches!(
            &available[0].frame,
            Frame::Metrics(metrics)
                if metrics.event == "pi_available_models"
                    && metrics.data["models"][0]["provider"] == "anthropic"
        ));
        assert_eq!(switched.len(), 1);
        assert!(matches!(
            &switched[0].frame,
            Frame::Metrics(metrics)
                if metrics.event == "pi_model_changed"
                    && metrics.data["id"] == "claude-sonnet-4"
        ));
    }

    #[test]
    fn strips_think_blocks_from_text_deltas() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let frames = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"<think>the user pinged me</think>I'm checking the feed."}}"#,
            )
            .unwrap();

        assert_eq!(frames.len(), 1);
        assert!(matches!(
            &frames[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "I'm checking the feed."
        ));
    }

    #[test]
    fn strips_think_blocks_split_across_text_deltas() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let opening = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"<think>secret reasoning"}}"#,
            )
            .unwrap();
        let closing = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"</think>Stand by."}}"#,
            )
            .unwrap();

        assert!(opening.is_empty());
        assert_eq!(closing.len(), 1);
        assert!(matches!(
            &closing[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "Stand by."
        ));
    }

    #[test]
    fn strips_leading_thought_trace_before_brain_frames() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let frames = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","channel":"thought","delta":"User just said \"Hey.\" - casual greeting, no specific request. I should respond warmly."}}"#,
            )
            .unwrap();

        assert!(frames.is_empty());
    }

    #[test]
    fn reasoning_delta_events_are_ignored() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let frames = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"reasoning_delta","content":"The user wants to know what's on their calendar. Let me check."}}"#,
            )
            .unwrap();

        assert!(frames.is_empty());

        let spoken = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Here's your agenda for today: first, Kita at eight."}}"#,
            )
            .unwrap();
        assert_eq!(spoken.len(), 1);
        assert!(matches!(
            &spoken[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "Here's your agenda for today: first, Kita at eight."
        ));
    }

    #[test]
    fn drops_provider_reasoning_fields_without_visible_content() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let qwen = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","reasoning":"Here's a thinking process.","content":null}}"#,
            )
            .unwrap();
        let gemma = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","reasoning_content":"Internal reasoning.","content":null}}"#,
            )
            .unwrap();

        assert!(qwen.is_empty());
        assert!(gemma.is_empty());
    }

    #[test]
    fn maps_pi_rpc_tool_and_error_events_to_brain_frames() {
        let session_id = SessionId::new();
        let tool = rpc_line_to_frames(
            &session_id,
            r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"read","message":{"path":"README.md"}}"#,
        )
        .unwrap();
        assert!(matches!(
            &tool[0].frame,
            Frame::Brain(BrainFrame::ToolCall { name, .. }) if name == "read"
        ));

        let err = rpc_line_to_frames(&session_id, r#"{"type":"error","error":"boom"}"#).unwrap();
        assert!(matches!(
            &err[0].frame,
            Frame::Brain(BrainFrame::Error { message }) if message == "boom"
        ));
    }
}
