//! Cold-start boundary: a spawned process is not usable until empty ready.
use std::{process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
};

use super::{read_worker_frame, QwenWorkerConfig, WORKER_OUTPUT_ERROR, WORKER_OUTPUT_READY};

pub(super) struct WorkerProcess {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub buffer: Vec<u8>,
}

pub(super) async fn spawn_ready_worker(config: &QwenWorkerConfig) -> Result<WorkerProcess> {
    let mut child = Command::new(&config.command)
        .args(&config.args)
        .current_dir(&config.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("launch {} TTS worker {}", config.backend, config.command))?;
    let stdin = child.stdin.take().context("TTS worker stdin unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .context("TTS worker stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("TTS worker stderr unavailable")?;
    let stderr = tokio::spawn(drain_stderr(stderr, config.backend.clone()));
    let mut buffer = Vec::new();
    if let Err(error) = wait_ready(config, &mut stdout, &mut buffer).await {
        let _ = child.kill().await;
        let tail = match tokio::time::timeout(Duration::from_secs(1), stderr).await {
            Ok(Ok(tail)) => String::from_utf8_lossy(&tail).trim().to_string(),
            _ => "<stderr unavailable>".to_string(),
        };
        let message = format!(
            "{} TTS worker startup failed: {error:#}; stderr tail: {tail}",
            config.backend
        );
        return Err(error.context(message));
    }
    Ok(WorkerProcess {
        child,
        stdin,
        stdout,
        buffer,
    })
}

async fn wait_ready(
    config: &QwenWorkerConfig,
    stdout: &mut ChildStdout,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    let ready = tokio::time::timeout(config.startup_timeout, read_worker_frame(stdout, buffer))
        .await
        .context("TTS worker ready timeout")??
        .context("TTS worker exited before ready")?;
    match ready.frame_type {
        WORKER_OUTPUT_READY => Ok(()),
        WORKER_OUTPUT_ERROR => bail!(
            "TTS worker startup error: {}",
            String::from_utf8_lossy(&ready.payload)
        ),
        other => bail!("TTS worker expected ready, received frame {other}"),
    }
}

async fn drain_stderr(mut stderr: ChildStderr, backend: String) -> Vec<u8> {
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 8192];
    // Drain continuously, retaining only a bounded diagnostic tail for startup
    // failures. Model/library chatter must never backpressure the stdout pipe.
    while let Ok(count) = stderr.read(&mut chunk).await {
        if count == 0 {
            break;
        }
        tracing::debug!(%backend, stderr = %String::from_utf8_lossy(&chunk[..count]), "TTS worker stderr");
        tail.extend_from_slice(&chunk[..count]);
        let excess = tail.len().saturating_sub(16 * 1024);
        tail.drain(..excess);
    }
    tail
}
