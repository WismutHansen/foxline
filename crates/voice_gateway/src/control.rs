//! Loopback control channel for the `switch_agent` Pi extension tool
//! (`crates/voice_gateway/extensions/switch-agent/index.ts`).
//!
//! The extension's `execute()` runs in-process inside the Pi child process
//! and talks to the gateway directly over raw TCP (one JSON request line, one
//! JSON response line) rather than routing through the Pi RPC stdout stream
//! the gateway already parses. This keeps the gateway out of the business of
//! interpreting Pi's tool calls: `switch_agent` is just a tool with a direct
//! side effect, like any other extension tool.
//!
//! Requests are authenticated by a token equal to the calling Pi process's
//! own `BrainIdentity::session_name()`, handed to it as `FOXLINE_CONTROL_TOKEN`
//! at launch. That value is deterministic (a pure function of agent + loadout
//! + frontend capability profile), so it survives `BrainPool` reuse of an
//! already-warm process without needing a fresh per-connection secret.

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, Mutex},
};
use tracing::{debug, warn};

/// A workspace/persona switch requested by the `switch_agent` tool, routed to
/// whichever `handle_connection` loop currently owns the calling Pi process.
#[derive(Debug)]
pub struct SwitchAgentRequest {
    pub workspace: String,
    pub persona: Option<String>,
    pub reply: oneshot::Sender<SwitchAgentReply>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchAgentReply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loadout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SwitchAgentReply {
    pub fn ok(workspace: String, agent: String, persona: String, loadout: String) -> Self {
        Self {
            ok: true,
            workspace: Some(workspace),
            agent: Some(agent),
            persona: Some(persona),
            loadout: Some(loadout),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            workspace: None,
            agent: None,
            persona: None,
            loadout: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SwitchAgentWireRequest {
    token: String,
    workspace: String,
    #[serde(default)]
    persona: Option<String>,
}

/// Process-wide registry of live WS sessions reachable by the `switch_agent`
/// control channel: keyed by the calling Pi process's own session name, value
/// is the channel the owning `handle_connection` loop is listening on.
#[derive(Clone, Default)]
pub struct ControlRegistry(Arc<Mutex<HashMap<String, mpsc::UnboundedSender<SwitchAgentRequest>>>>);

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, token: String, tx: mpsc::UnboundedSender<SwitchAgentRequest>) {
        self.0.lock().await.insert(token, tx);
    }

    pub async fn unregister(&self, token: &str) {
        self.0.lock().await.remove(token);
    }

    async fn dispatch(&self, request: SwitchAgentWireRequest) -> SwitchAgentReply {
        let tx = self.0.lock().await.get(&request.token).cloned();
        let Some(tx) = tx else {
            return SwitchAgentReply::err("unknown or expired control token");
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        if tx
            .send(SwitchAgentRequest {
                workspace: request.workspace,
                persona: request.persona,
                reply: reply_tx,
            })
            .is_err()
        {
            return SwitchAgentReply::err("session is no longer accepting switch requests");
        }
        match tokio::time::timeout(Duration::from_secs(30), reply_rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => SwitchAgentReply::err("session dropped the switch request"),
            Err(_) => SwitchAgentReply::err("switch_agent timed out"),
        }
    }
}

/// Binds a loopback-only JSON-lines control listener: one connection, one
/// request line, one response line, then close. No HTTP framework — this
/// matches `ws.rs`'s existing raw-TCP style for a single fixed-purpose
/// endpoint that only ever talks to Pi extensions running on the same host.
pub async fn run_control_listener(registry: ControlRegistry) -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind switch_agent control listener")?;
    let addr = listener
        .local_addr()
        .context("read switch_agent control listener addr")?;
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(err) => {
                    warn!(error = %err, "control listener accept failed");
                    continue;
                }
            };
            let registry = registry.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_control_connection(stream, registry).await {
                    debug!(error = %err, "control connection failed");
                }
            });
        }
    });
    Ok(addr)
}

async fn handle_control_connection(stream: TcpStream, registry: ControlRegistry) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let Some(line) = lines
        .next_line()
        .await
        .context("read switch_agent control request line")?
    else {
        return Ok(());
    };
    let reply = match serde_json::from_str::<SwitchAgentWireRequest>(&line) {
        Ok(request) => registry.dispatch(request).await,
        Err(err) => SwitchAgentReply::err(format!("invalid switch_agent request: {err}")),
    };
    let mut payload = serde_json::to_vec(&reply).context("serialize switch_agent reply")?;
    payload.push(b'\n');
    writer
        .write_all(&payload)
        .await
        .context("write switch_agent reply")?;
    writer.flush().await.context("flush switch_agent reply")?;
    Ok(())
}
