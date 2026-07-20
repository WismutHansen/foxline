use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(name = "foxline-voice-gateway")]
#[command(about = "Rust Voice Gateway runtime for Foxline voice sessions")]
pub struct Cli {
    #[arg(long, env = "FOXLINE_GATEWAY_CONFIG")]
    pub config: Option<PathBuf>,

    #[arg(long, env = "FOXLINE_GATEWAY_BIND")]
    pub bind: Option<String>,

    #[arg(long, env = "FOXLINE_GATEWAY_TRACE_DIR")]
    pub trace_dir: Option<PathBuf>,

    #[arg(long, env = "FOXLINE_GATEWAY_DEBUG_TRACES")]
    pub debug_traces: Option<bool>,

    #[arg(long)]
    pub print_config_schema: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GatewayConfig {
    /// Socket address the gateway listens on. `127.0.0.1:<port>` (default)
    /// only accepts local connections. `0.0.0.0:<port>` accepts connections
    /// from the LAN and from a Tailscale interface alike (Tailscale exposes
    /// itself as a regular network interface, so no separate Tailscale-only
    /// mode is needed) — pair it with `auth.token` if the machine is reachable
    /// beyond a trusted network.
    pub bind: String,
    pub trace_dir: PathBuf,
    pub debug_traces: bool,
    pub session_idle_timeout_ms: u64,
    pub brain: BrainConfig,
    pub frontend: FrontendConfig,
    pub turn: TurnStrategyConfig,
    pub loadouts: LoadoutConfig,
    pub auth: AuthConfig,
    pub workspaces: WorkspacesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrainConfig {
    pub pi_command: String,
    pub idle_timeout_ms: u64,
    pub prewarm: bool,
    pub no_extensions: bool,
    pub no_context_files: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FrontendConfig {
    pub require_capability_declaration: bool,
    pub max_audio_frame_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TurnStrategyConfig {
    pub silence_timeout_ms: u64,
    pub min_speech_duration_ms: u64,
    pub max_utterance_duration_ms: u64,
    pub barge_in_confirmation_window_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LoadoutConfig {
    pub default_name: String,
    pub bundled_extensions_dir: PathBuf,
}

/// Shared-secret handshake auth. `token: None` (the default) preserves
/// today's zero-friction behavior (no auth check at all) — set a token only
/// once the gateway is reachable beyond localhost (LAN or Tailscale bind).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AuthConfig {
    pub token: Option<String>,
}

/// Gateway-owned workspace registry so a frontend can boot without any
/// build-time env vars: `default` names the entry used when the client
/// doesn't request a specific workspace, `registry` maps a short id to the
/// workspace path/agent/persona/loadout to use.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkspacesConfig {
    pub default: Option<String>,
    pub registry: BTreeMap<String, WorkspaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    pub agent: Option<String>,
    pub persona: Option<String>,
    pub loadout: Option<String>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8780".to_string(),
            trace_dir: default_state_dir().join("traces").join("rust-gateway"),
            debug_traces: false,
            session_idle_timeout_ms: 300_000,
            brain: BrainConfig::default(),
            frontend: FrontendConfig::default(),
            turn: TurnStrategyConfig::default(),
            loadouts: LoadoutConfig::default(),
            auth: AuthConfig::default(),
            workspaces: WorkspacesConfig::default(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { token: None }
    }
}

impl Default for WorkspacesConfig {
    fn default() -> Self {
        Self {
            default: None,
            registry: BTreeMap::new(),
        }
    }
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            pi_command: "pi".to_string(),
            idle_timeout_ms: 300_000,
            prewarm: false,
            no_extensions: true,
            no_context_files: true,
        }
    }
}

impl Default for FrontendConfig {
    fn default() -> Self {
        Self {
            require_capability_declaration: true,
            max_audio_frame_bytes: 32 * 1024,
        }
    }
}

impl Default for TurnStrategyConfig {
    fn default() -> Self {
        Self {
            silence_timeout_ms: 650,
            min_speech_duration_ms: 180,
            max_utterance_duration_ms: 30_000,
            barge_in_confirmation_window_ms: 450,
        }
    }
}

impl Default for LoadoutConfig {
    fn default() -> Self {
        Self {
            default_name: "default".to_string(),
            bundled_extensions_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extensions"),
        }
    }
}

impl GatewayConfig {
    pub fn load(cli: &Cli) -> Result<Self> {
        let default_path = default_config_path();
        ensure_default_config(&default_path)?;

        let mut builder = config::Config::builder();
        let path = cli.config.clone().unwrap_or(default_path);
        if path.exists() {
            builder = builder.add_source(config::File::from(path).required(false));
        }

        let mut loaded: GatewayConfig = builder
            .build()
            .context("build gateway config")?
            .try_deserialize()
            .context("deserialize gateway config")?;

        if let Some(bind) = &cli.bind {
            loaded.bind = bind.clone();
        }
        if let Some(trace_dir) = &cli.trace_dir {
            loaded.trace_dir = trace_dir.clone();
        }
        if let Some(debug_traces) = cli.debug_traces {
            loaded.debug_traces = debug_traces;
        }

        Ok(loaded)
    }

    pub fn schema_json() -> Result<String> {
        serde_json::to_string_pretty(&schema_for!(GatewayConfig)).context("serialize config schema")
    }
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("foxline")
        .join("config.toml")
}

fn default_state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("foxline")
}

fn ensure_default_config(path: &PathBuf) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    fs::write(path, default_config_toml())
        .with_context(|| format!("write default config {}", path.display()))?;
    Ok(())
}

fn default_config_toml() -> &'static str {
    r#"# Foxline Voice Gateway defaults.
# "127.0.0.1:<port>" (default) only accepts local connections. Use
# "0.0.0.0:<port>" to accept connections from the LAN and/or a Tailscale
# interface (Tailscale appears as a regular network interface, so a single
# 0.0.0.0 bind covers both) — pair with [auth] token below if reachable
# beyond a trusted network.
bind = "127.0.0.1:8780"
debug_traces = false
session_idle_timeout_ms = 300000

[brain]
pi_command = "pi"
idle_timeout_ms = 300000
prewarm = false
no_extensions = true
no_context_files = true

[frontend]
require_capability_declaration = true
max_audio_frame_bytes = 32768

[turn]
silence_timeout_ms = 650
min_speech_duration_ms = 180
max_utterance_duration_ms = 30000
barge_in_confirmation_window_ms = 450

[loadouts]
default_name = "default"

# Optional shared-secret handshake auth. Unset (the default) disables the
# check entirely — safe for a loopback-only bind. Set a token once `bind`
# is reachable beyond localhost:
# [auth]
# token = "change-me"

# Optional gateway-owned workspace registry. When present, the frontend can
# read this instead of hardcoding VITE_FOXLINE_GATEWAY_* build-time env vars.
# [workspaces]
# default = "main"
#
# [workspaces.registry.main]
# path = "/Users/you/projects/my-repo"
# agent = "campbell"
# persona = "campbell"
# loadout = "default"
"#
}
