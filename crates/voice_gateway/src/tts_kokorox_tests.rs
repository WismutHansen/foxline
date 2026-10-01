//! Opt-in real-model proof of Persona -> launch config -> native subprocess -> gateway frames.
use super::kokorox_worker_config;
use crate::{
    frame::{AudioFrame, Frame, SessionId, TtsFrame},
    loadout::{AdapterLoadout, KokoroxWorkerLoadout},
    persona::PersonaRegistry,
    trace::TraceWriter,
    tts::{QwenWorkerTtsAdapter, TtsAdapter},
};
use std::{
    env, fs,
    path::Path,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::time::timeout;

fn install_asset(source: &Path, destination: &Path) {
    if fs::hard_link(source, destination).is_err() {
        fs::copy(source, destination).unwrap();
    }
}

async fn collect(
    adapter: &mut QwenWorkerTtsAdapter,
    session: &SessionId,
    start: Instant,
) -> (usize, u128, f64) {
    let mut bytes = 0;
    let mut first_pcm = None;
    let mut nonzero = false;
    loop {
        let event = timeout(Duration::from_secs(120), adapter.next_frame())
            .await
            .unwrap()
            .expect("worker output");
        assert_eq!(
            &event.session_id, session,
            "cancelled request escaped generation fence"
        );
        match event.frame {
            Frame::Tts(TtsFrame::AudioStart {
                sample_rate_hz,
                channels,
            }) => assert_eq!((sample_rate_hz, channels), (24000, 1)),
            Frame::Audio(AudioFrame::OutputPcm {
                sample_rate_hz,
                channels,
                bytes: pcm,
            }) => {
                assert_eq!((sample_rate_hz, channels), (24000, 1));
                assert_eq!(pcm.len() % 2, 0);
                if !pcm.is_empty() {
                    first_pcm.get_or_insert(start.elapsed().as_millis());
                }
                nonzero |= pcm.iter().any(|&byte| byte != 0);
                bytes += pcm.len();
            }
            Frame::Tts(TtsFrame::AudioDone) => break,
            Frame::Tts(TtsFrame::Error { message }) => panic!("native worker: {message}"),
            _ => {}
        }
    }
    assert!(bytes > 0 && nonzero, "missing/non-silent PCM");
    let rtf = start.elapsed().as_secs_f64() / (bytes as f64 / 48000.0);
    (bytes, first_pcm.expect("first PCM"), rtf)
}

#[tokio::test]
#[ignore = "requires KOKOROX_TEST_WORKER, KOKOROX_TEST_MODEL, KOKOROX_TEST_VOICES"]
async fn real_kokorox_persona_worker_turn_cancel_and_recovery() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join(".foxline/personas/native");
    fs::create_dir_all(root.join("voice")).unwrap();
    install_asset(
        Path::new(&env::var("KOKOROX_TEST_MODEL").unwrap()),
        &root.join("voice/model.onnx"),
    );
    install_asset(
        Path::new(&env::var("KOKOROX_TEST_VOICES").unwrap()),
        &root.join("voice/voices.npz"),
    );
    let voice = env::var("KOKOROX_TEST_VOICE").unwrap_or_else(|_| "af_heart".into());
    let language = env::var("KOKOROX_TEST_LANGUAGE").unwrap_or_else(|_| "en-us".into());
    let manifest = format!("[voice.kokorox]\nmodel='voice/model.onnx'\nvoices='voice/voices.npz'\nvoice={}\nlanguage={}\nspeed=1.0\n", toml::Value::String(voice), toml::Value::String(language));
    fs::write(root.join("persona.toml"), manifest).unwrap();
    let persona = PersonaRegistry::new_with_data_root(temp.path(), temp.path().join("data"))
        .resolve("native", temp.path())
        .unwrap();
    let adapters = AdapterLoadout {
        tts: "kokorox".into(),
        kokorox: Some(KokoroxWorkerLoadout {
            worker: env::var("KOKOROX_TEST_WORKER").unwrap(),
        }),
        ..Default::default()
    };
    let config = kokorox_worker_config(&adapters, &persona, temp.path(), None).unwrap();
    let trace_dir = temp.path().join("traces");
    let mut adapter = QwenWorkerTtsAdapter::new(config)
        .with_trace(TraceWriter::create(trace_dir.clone(), "native").unwrap());
    assert_eq!(adapter.backend_name(), "kokorox");
    let startup = Instant::now();
    adapter.prewarm().await.unwrap();
    println!("native worker startup_ms={}", startup.elapsed().as_millis());
    let text = env::var("KOKOROX_TEST_TEXT")
        .unwrap_or_else(|_| "Good morning. This is a real worker integration test.".into());
    let first = SessionId::new();
    let start = Instant::now();
    adapter.speak(first.clone(), text.clone()).await.unwrap();
    let (bytes, ttfa, rtf) = collect(&mut adapter, &first, start).await;
    println!("native initial: pcm_bytes={bytes} ttfa_ms={ttfa} rtf={rtf:.4}");
    let cancelled = SessionId::new();
    adapter
        .speak(cancelled.clone(), text.repeat(20))
        .await
        .unwrap();
    let start = timeout(Duration::from_secs(120), adapter.next_frame())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        start.frame,
        Frame::Tts(TtsFrame::AudioStart { .. })
    ));
    adapter.cancel(cancelled).await.unwrap();
    let recovery = SessionId::new();
    let start = Instant::now();
    adapter.speak(recovery.clone(), text).await.unwrap();
    let (bytes, ttfa, rtf) = collect(&mut adapter, &recovery, start).await;
    println!("native recovery: pcm_bytes={bytes} ttfa_ms={ttfa} rtf={rtf:.4}");
    assert!(adapter.try_next_frame().is_none());
    adapter.shutdown().await.unwrap();
    assert!(adapter.next_frame().await.is_none());
    let mut invalid = persona;
    invalid.kokorox_voice.as_mut().unwrap().voice = "missing-voice-for-test".into();
    let config = kokorox_worker_config(&adapters, &invalid, temp.path(), None).unwrap();
    let mut invalid_worker = QwenWorkerTtsAdapter::new(config);
    let error = invalid_worker.prewarm().await.unwrap_err();
    assert!(
        error.to_string().contains("voice"),
        "lost native startup diagnostic: {error:#}"
    );
    invalid_worker.shutdown().await.unwrap();
    println!("native invalid-voice startup rejected with diagnostic");
    let traces = fs::read_to_string(trace_dir.join("native.jsonl")).unwrap();
    assert_eq!(
        traces
            .lines()
            .filter(|line| line.contains("tts_request_start") && line.contains("kokorox"))
            .count(),
        3
    );
    assert_eq!(
        traces
            .lines()
            .filter(|line| line.contains("tts_cancel_sent"))
            .count(),
        1
    );
}
