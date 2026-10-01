use super::*;

fn queued_adapter() -> (QwenWorkerTtsAdapter, mpsc::Sender<(u64, FrameEnvelope)>) {
    let mut adapter = QwenWorkerTtsAdapter::new(QwenWorkerConfig::new(
        "/nonexistent/foxline-test-worker",
        vec![],
        ".",
    ));
    let (tx, rx) = mpsc::channel(16);
    adapter.events = Some(rx);
    (adapter, tx)
}

fn output(session: &SessionId) -> FrameEnvelope {
    FrameEnvelope::new(
        session.clone(),
        Frame::Audio(AudioFrame::OutputPcm {
            sample_rate_hz: 24_000,
            channels: 1,
            bytes: Bytes::from_static(b"old audio"),
        }),
    )
}

#[tokio::test]
async fn cancellation_fences_queued_audio_and_completion_even_after_worker_done() {
    let (mut adapter, tx) = queued_adapter();
    let session = SessionId::new();
    let request = ActiveTtsRequest {
        generation: 0,
        session_id: session.clone(),
        sample_rate_hz: 24_000,
    };
    for kind in [
        WORKER_OUTPUT_AUDIO_START,
        WORKER_OUTPUT_AUDIO_CHUNK,
        WORKER_OUTPUT_AUDIO_DONE,
        WORKER_OUTPUT_ERROR,
    ] {
        let worker_frame = WorkerFrame {
            frame_type: kind,
            request_id: 1,
            payload: Bytes::new(),
        };
        for frame in qwen_output_to_frames(&session, &request, &worker_frame) {
            tx.send((request.generation, frame)).await.unwrap();
        }
    }
    // The reader removes active requests as soon as it sees Done, not when
    // the gateway has consumed their output. That is the original race.
    assert!(adapter.active.lock().await.is_empty());
    adapter.cancel(session.clone()).await.unwrap();
    assert!(
        adapter.try_next_frame().is_none(),
        "queued output survived cancellation"
    );

    // A producer may already have cloned the old request and enqueue AFTER
    // cancellation. Clearing the queue alone is insufficient.
    tx.send((request.generation, output(&session)))
        .await
        .unwrap();
    let fresh = FrameEnvelope::new(session, Frame::Tts(TtsFrame::AudioDone));
    tx.send((adapter.generation, fresh.clone())).await.unwrap();
    assert_eq!(adapter.try_next_frame(), Some(fresh));
    assert!(adapter.try_next_frame().is_none());
}

#[tokio::test]
async fn asynchronous_receive_skips_late_old_output() {
    let (mut adapter, tx) = queued_adapter();
    let session = SessionId::new();
    let old_generation = adapter.generation;
    adapter.cancel(session.clone()).await.unwrap();
    tx.send((old_generation, output(&session))).await.unwrap();
    let fresh = output(&SessionId::new());
    tx.send((adapter.generation, fresh.clone())).await.unwrap();
    drop(tx);
    assert_eq!(adapter.next_frame().await, Some(fresh));
    assert_eq!(adapter.next_frame().await, None);
}

#[tokio::test]
async fn cancellation_fences_output_even_when_worker_cancel_write_fails() {
    let (mut adapter, tx) = queued_adapter();
    let session = SessionId::new();
    adapter.active.lock().await.insert(
        1,
        ActiveTtsRequest {
            generation: 0,
            session_id: session.clone(),
            sample_rate_hz: 24_000,
        },
    );
    tx.send((0, output(&session))).await.unwrap();
    assert!(adapter.cancel(session).await.is_err());
    assert!(adapter.active.lock().await.is_empty());
    assert!(adapter.try_next_frame().is_none());
}

#[tokio::test]
async fn shutdown_discards_active_requests() {
    let (mut adapter, _) = queued_adapter();
    adapter.active.lock().await.insert(
        1,
        ActiveTtsRequest {
            generation: 0,
            session_id: SessionId::new(),
            sample_rate_hz: 24_000,
        },
    );
    adapter.shutdown().await.unwrap();
    assert!(adapter.active.lock().await.is_empty());
    assert!(adapter.try_next_frame().is_none());
}
