use std::{path::PathBuf, process::Command, sync::OnceLock, time::Duration};

use foxline_voice_gateway::{
    frame::{AudioFrame, Frame, SessionId, TtsFrame},
    tts::{QwenWorkerConfig, QwenWorkerTtsAdapter, TtsAdapter},
};
use tempfile::TempDir;
use tokio::time::timeout;

#[test]
fn loadout_schema_includes_native_worker_configuration() {
    let output = Command::new(env!("CARGO_BIN_EXE_foxline-voice-gateway"))
        .arg("--print-loadout-schema")
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        schema["definitions"]["KokoroxWorkerLoadout"]["properties"]["worker"]["type"],
        "string"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_prewarm_future_does_not_leave_worker_running() {
    let pid_file = fixture()._dir.path().join("cancelled-startup.pid");
    let mut config = QwenWorkerConfig::new(
        fixture().binary.to_string_lossy(),
        vec!["timeout".into(), pid_file.to_string_lossy().into_owned()],
        fixture()._dir.path(),
    );
    config.startup_timeout = Duration::from_secs(10);
    let mut adapter = QwenWorkerTtsAdapter::new(config);
    {
        let warm = adapter.prewarm();
        tokio::pin!(warm);
        // Wait for evidence of process entry, rather than racing OS cold exec.
        timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    result = &mut warm => panic!("unexpected prewarm completion: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        if pid_file.is_file() { break; }
                    }
                }
            }
        })
        .await
        .unwrap();
    } // Dropping the pending future must kill its local child.
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let exited = timeout(Duration::from_secs(5), async {
        loop {
            let alive = Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(exited.is_ok(), "worker still alive after cancelled prewarm");
}

struct Fixture {
    _dir: TempDir,
    binary: PathBuf,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        let binary = dir
            .path()
            .join(format!("tts-worker{}", std::env::consts::EXE_SUFFIX));
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tts_worker.rs");
        let output = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
            .args(["--edition=2021", "-o"])
            .arg(&binary)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Fixture { _dir: dir, binary }
    })
}

fn adapter(mode: &str) -> QwenWorkerTtsAdapter {
    let mut config = QwenWorkerConfig::new(
        fixture().binary.to_string_lossy(),
        vec![mode.into()],
        fixture()._dir.path(),
    );
    config.startup_timeout = if mode == "timeout" {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(5)
    };
    QwenWorkerTtsAdapter::new(config)
}

async fn next(adapter: &mut QwenWorkerTtsAdapter) -> foxline_voice_gateway::frame::FrameEnvelope {
    timeout(Duration::from_secs(5), adapter.next_frame())
        .await
        .unwrap()
        .expect("worker output")
}

async fn successful_turn(adapter: &mut QwenWorkerTtsAdapter, session: SessionId) {
    adapter
        .speak(session.clone(), "Guten Morgen, Straße.".into())
        .await
        .unwrap();
    let mut pcm = false;
    loop {
        let event = next(adapter).await;
        assert_eq!(event.session_id, session, "stale session leaked");
        match event.frame {
            Frame::Tts(TtsFrame::AudioStart {
                sample_rate_hz,
                channels,
            }) => assert_eq!((sample_rate_hz, channels), (48000, 1)),
            Frame::Audio(AudioFrame::OutputPcm {
                sample_rate_hz,
                channels,
                bytes,
            }) => {
                assert_eq!((sample_rate_hz, channels), (48000, 1));
                assert_eq!(&bytes[..], &[1, 0, 255, 127]);
                pcm = true;
            }
            Frame::Tts(TtsFrame::AudioDone) => break,
            Frame::Tts(TtsFrame::Error { message }) => panic!("{message}"),
            _ => {}
        }
    }
    assert!(pcm);
}

#[tokio::test]
async fn partial_frames_raw_utf8_and_worker_rate_survive_multiple_turns() {
    let mut adapter = adapter("normal");
    adapter.prewarm().await.unwrap();
    successful_turn(&mut adapter, SessionId::new()).await;
    successful_turn(&mut adapter, SessionId::new()).await;
    adapter.shutdown().await.unwrap();
    assert!(adapter.next_frame().await.is_none());
}

#[tokio::test]
async fn shutdown_and_restart_keep_process_request_maps_independent() {
    let mut adapter = adapter("normal");
    successful_turn(&mut adapter, SessionId::new()).await;
    adapter.shutdown().await.unwrap();
    successful_turn(&mut adapter, SessionId::new()).await;
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn ready_wait_drains_stderr_without_blocking() {
    let mut adapter = adapter("stderr");
    adapter.prewarm().await.unwrap();
    successful_turn(&mut adapter, SessionId::new()).await;
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn startup_errors_invalid_ready_and_timeouts_are_not_successful_prewarm() {
    for (mode, expected) in [
        ("startup-error", "missing model asset"),
        ("stderr-error", "model stderr error"),
        ("invalid-ready", "invalid TTS worker output frame"),
        ("timeout", "ready timeout"),
    ] {
        let mut adapter = adapter(mode);
        let error = adapter.prewarm().await.unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{mode}: {error:#}");
        assert!(error.to_string().len() < 17 * 1024, "unbounded stderr tail");
        assert!(adapter.next_frame().await.is_none());
        adapter.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_output_or_eof_terminates_pending_request_with_error() {
    for mode in ["crash", "truncated", "odd-pcm", "wrong-order"] {
        let mut adapter = adapter(mode);
        adapter
            .speak(SessionId::new(), "fixture".into())
            .await
            .unwrap();
        loop {
            let frame = next(&mut adapter).await;
            match frame.frame {
                Frame::Tts(TtsFrame::Error { message }) => {
                    assert!(message.contains("TTS worker"));
                    break;
                }
                Frame::Tts(TtsFrame::AudioStart { .. }) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        adapter.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn per_request_error_does_not_poison_subsequent_turn() {
    let mut adapter = adapter("request-error");
    adapter
        .speak(SessionId::new(), "fixture".into())
        .await
        .unwrap();
    assert!(matches!(
        next(&mut adapter).await.frame,
        Frame::Tts(TtsFrame::Error { .. })
    ));
    successful_turn(&mut adapter, SessionId::new()).await;
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancel_fences_late_worker_output_and_allows_subsequent_turn() {
    let mut adapter = adapter("late");
    let old = SessionId::new();
    adapter.speak(old.clone(), "fixture".into()).await.unwrap();
    assert!(matches!(
        next(&mut adapter).await.frame,
        Frame::Tts(TtsFrame::AudioStart { .. })
    ));
    adapter.cancel(old).await.unwrap();
    successful_turn(&mut adapter, SessionId::new()).await;
    assert!(adapter.try_next_frame().is_none());
    adapter.shutdown().await.unwrap();
}
