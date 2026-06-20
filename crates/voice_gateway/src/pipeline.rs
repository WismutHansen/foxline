use anyhow::Result;
use async_trait::async_trait;

use crate::frame::{Frame, FrameEnvelope, InterruptReason, SessionId, TurnFrame};

#[async_trait]
pub trait FrameProcessor: Send {
    async fn process(&mut self, frame: FrameEnvelope) -> Result<Vec<FrameEnvelope>>;
}

pub struct LinearPipeline {
    processors: Vec<Box<dyn FrameProcessor>>,
}

impl LinearPipeline {
    pub fn new(processors: Vec<Box<dyn FrameProcessor>>) -> Self {
        Self { processors }
    }

    pub async fn process(&mut self, frame: FrameEnvelope) -> Result<Vec<FrameEnvelope>> {
        let mut frames = vec![frame];
        for processor in &mut self.processors {
            let mut next = Vec::new();
            for frame in frames {
                next.extend(processor.process(frame).await?);
            }
            frames = next;
        }
        Ok(frames)
    }
}

pub struct PassthroughProcessor;

#[async_trait]
impl FrameProcessor for PassthroughProcessor {
    async fn process(&mut self, frame: FrameEnvelope) -> Result<Vec<FrameEnvelope>> {
        Ok(vec![frame])
    }
}

pub struct InterruptFanoutProcessor;

#[async_trait]
impl FrameProcessor for InterruptFanoutProcessor {
    async fn process(&mut self, frame: FrameEnvelope) -> Result<Vec<FrameEnvelope>> {
        if matches!(
            frame.frame,
            Frame::Turn(TurnFrame::Interrupted {
                reason: InterruptReason::FrontendBargeIn
                    | InterruptReason::UserCancel
                    | InterruptReason::VadSpeechStart
            })
        ) {
            let session_id = frame.session_id.clone();
            Ok(vec![
                frame,
                FrameEnvelope::new(session_id, Frame::Tts(crate::frame::TtsFrame::Cancel)),
            ])
        } else {
            Ok(vec![frame])
        }
    }
}

pub fn default_pipeline() -> LinearPipeline {
    LinearPipeline::new(vec![
        Box::new(InterruptFanoutProcessor),
        Box::new(PassthroughProcessor),
    ])
}

pub fn lifecycle_started(session_id: SessionId) -> FrameEnvelope {
    FrameEnvelope::new(
        session_id,
        Frame::Lifecycle(crate::frame::LifecycleFrame::SessionStarted),
    )
}

#[cfg(test)]
mod tests {
    use crate::frame::{Frame, FrameEnvelope, InterruptReason, SessionId, TtsFrame, TurnFrame};

    use super::{default_pipeline, LinearPipeline, PassthroughProcessor};

    #[tokio::test]
    async fn passthrough_keeps_frame_order() {
        let session_id = SessionId::new();
        let frame = FrameEnvelope::new(session_id, Frame::Turn(TurnFrame::UserStarted));
        let mut pipeline = LinearPipeline::new(vec![Box::new(PassthroughProcessor)]);

        let out = pipeline.process(frame.clone()).await.unwrap();

        assert_eq!(out, vec![frame]);
    }

    #[tokio::test]
    async fn interruption_propagates_tts_cancel() {
        let session_id = SessionId::new();
        let frame = FrameEnvelope::new(
            session_id,
            Frame::Turn(TurnFrame::Interrupted {
                reason: InterruptReason::FrontendBargeIn,
            }),
        );
        let mut pipeline = default_pipeline();

        let out = pipeline.process(frame.clone()).await.unwrap();

        assert_eq!(out[0], frame);
        assert!(matches!(out[1].frame, Frame::Tts(TtsFrame::Cancel)));
    }
}
