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
    frame::{BrainFrame, Frame, FrameEnvelope, SessionId},
    loadout::ResolvedLoadout,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BrainIdentity {
    pub agent: String,
    pub workspace: PathBuf,
    pub loadout: String,
    pub frontend_capability_hash: String,
}

impl BrainIdentity {
    pub fn new(
        agent: impl Into<String>,
        workspace: impl Into<PathBuf>,
        loadout: impl Into<String>,
        frontend_capability_profile: &Value,
    ) -> Self {
        Self {
            agent: agent.into(),
            workspace: workspace.into(),
            loadout: loadout.into(),
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
    content: Option<String>,
}

pub fn rpc_line_to_frames(session_id: &SessionId, line: &str) -> Result<Vec<FrameEnvelope>> {
    let event: RpcEvent = serde_json::from_str(line)?;
    Ok(RpcFrameMapper::default().event_to_frames(session_id, event))
}

#[derive(Default)]
struct RpcFrameMapper {
    assistant_content: String,
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
                    match assistant.event_type.as_str() {
                        "text_delta" => {
                            if let Some(delta) = assistant.delta {
                                self.assistant_content.push_str(&delta);
                                frames.push(FrameEnvelope::new(
                                    session_id.clone(),
                                    Frame::Brain(BrainFrame::TextDelta { text: delta }),
                                ));
                            }
                        }
                        "done" => {
                            self.assistant_content.clear();
                            frames.push(FrameEnvelope::new(
                                session_id.clone(),
                                Frame::Brain(BrainFrame::Done),
                            ));
                        }
                        _ => {}
                    }
                    if let Some(content) = assistant.content {
                        let delta = cumulative_suffix_delta(&self.assistant_content, &content);
                        if !delta.is_empty() {
                            frames.push(FrameEnvelope::new(
                                session_id.clone(),
                                Frame::Brain(BrainFrame::TextDelta {
                                    text: delta.to_string(),
                                }),
                            ));
                        }
                        self.assistant_content = content;
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

fn cumulative_suffix_delta<'a>(previous: &str, content: &'a str) -> &'a str {
    if previous.is_empty() {
        return content;
    }
    if content == previous {
        return "";
    }
    let mut split = 0;
    for ((prev_index, prev_ch), (content_index, content_ch)) in
        previous.char_indices().zip(content.char_indices())
    {
        if prev_ch != content_ch {
            break;
        }
        split = prev_index + prev_ch.len_utf8();
        debug_assert_eq!(split, content_index + content_ch.len_utf8());
    }
    if split == 0 {
        content
    } else {
        &content[split..]
    }
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

    use super::{rpc_line_to_frames, BrainIdentity, PiLaunch, RpcFrameMapper};

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

    #[test]
    fn pi_launch_includes_rpc_mode_identity_config_tools_and_extensions() {
        let dir = tempdir().unwrap();
        let resolved = resolved_loadout(dir.path().to_path_buf());
        let identity = BrainIdentity::new(
            "campbell",
            dir.path(),
            "default",
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
            .any(|pair| pair == ["--tools", "read,write"]));
        assert!(launch.args.contains(&"--extension".to_string()));
        assert!(launch
            .args
            .iter()
            .any(|arg| arg.ends_with(".foxline/extensions/frontend-tools")));
    }

    #[test]
    fn identity_is_bound_to_agent_loadout_workspace_and_frontend_profile() {
        let a = BrainIdentity::new("campbell", "/tmp/a", "default", &json!({ "tools": ["a"] }));
        let b = BrainIdentity::new("campbell", "/tmp/a", "default", &json!({ "tools": ["b"] }));
        let c = BrainIdentity::new("campbell", "/tmp/a", "field", &json!({ "tools": ["a"] }));

        assert_ne!(a, b);
        assert_ne!(a, c);
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
    fn maps_pi_rpc_cumulative_content_to_suffix_deltas() {
        let session_id = SessionId::new();
        let mut mapper = RpcFrameMapper::default();

        let first = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Copy that."}}"#,
            )
            .unwrap();
        let repeated_content = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":" Stay sharp.","content":"Copy that. Stay sharp."}}"#,
            )
            .unwrap();
        assert_eq!(mapper.assistant_content, "Copy that. Stay sharp.");
        let next_content = mapper
            .line_to_frames(
                &session_id,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","content":"Copy that. Stay sharp out there."}}"#,
            )
            .unwrap();

        assert!(matches!(
            &first[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == "Copy that."
        ));
        assert_eq!(repeated_content.len(), 1);
        assert!(matches!(
            &repeated_content[0].frame,
            Frame::Brain(BrainFrame::TextDelta { text }) if text == " Stay sharp."
        ));
        assert_eq!(next_content.len(), 1);
        match &next_content[0].frame {
            Frame::Brain(BrainFrame::TextDelta { text }) => assert_eq!(text, " out there."),
            frame => panic!("unexpected frame: {frame:?}"),
        }
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
