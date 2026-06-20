use anyhow::Result;
use async_trait::async_trait;
use time::OffsetDateTime;

use crate::{
    config::TurnStrategyConfig,
    frame::{Frame, FrameEnvelope, InterruptReason, SessionId, SttFrame, TurnFrame, VadFrame},
    pipeline::FrameProcessor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnState {
    Idle,
    Listening,
    UserSpeaking,
    AssistantSpeaking,
}

pub struct TurnManager {
    strategy: TurnStrategyConfig,
    state: TurnState,
    speech_started_at: Option<OffsetDateTime>,
    latest_frontend_hint: Option<bool>,
}

impl TurnManager {
    pub fn new(strategy: TurnStrategyConfig) -> Self {
        Self {
            strategy,
            state: TurnState::Idle,
            speech_started_at: None,
            latest_frontend_hint: None,
        }
    }

    fn emit(session_id: SessionId, frame: TurnFrame) -> FrameEnvelope {
        FrameEnvelope::new(session_id, Frame::Turn(frame))
    }

    fn speech_duration_ms(&self, now: OffsetDateTime) -> u64 {
        self.speech_started_at
            .and_then(|started| (now - started).whole_milliseconds().try_into().ok())
            .unwrap_or(0)
    }

    fn speech_duration_satisfies_minimum(&self, now: OffsetDateTime) -> bool {
        self.speech_duration_ms(now) >= self.strategy.min_speech_duration_ms
    }

    fn speech_duration_exceeds_maximum(&self, now: OffsetDateTime) -> bool {
        self.speech_duration_ms(now) >= self.strategy.max_utterance_duration_ms
    }
}

#[async_trait]
impl FrameProcessor for TurnManager {
    async fn process(&mut self, frame: FrameEnvelope) -> Result<Vec<FrameEnvelope>> {
        let mut out = Vec::with_capacity(3);
        let now = OffsetDateTime::now_utc();

        match &frame.frame {
            Frame::Lifecycle(crate::frame::LifecycleFrame::SessionStarted) => {
                self.state = TurnState::Listening;
                self.speech_started_at = None;
            }
            Frame::Vad(VadFrame::FrontendHint { speaking, .. }) => {
                self.latest_frontend_hint = Some(*speaking);
            }
            Frame::Vad(VadFrame::SpeechStarted) => {
                if matches!(self.state, TurnState::AssistantSpeaking) {
                    out.push(Self::emit(
                        frame.session_id.clone(),
                        TurnFrame::Interrupted {
                            reason: InterruptReason::VadSpeechStart,
                        },
                    ));
                }
                if !matches!(self.state, TurnState::UserSpeaking) {
                    self.state = TurnState::UserSpeaking;
                    self.speech_started_at = Some(now);
                    out.push(Self::emit(frame.session_id.clone(), TurnFrame::UserStarted));
                }
            }
            Frame::Vad(VadFrame::SpeechStopped) => {
                if matches!(self.state, TurnState::UserSpeaking)
                    && self.speech_duration_satisfies_minimum(now)
                {
                    self.state = TurnState::Listening;
                    self.speech_started_at = None;
                }
            }
            Frame::Stt(SttFrame::Partial { .. }) => {
                if matches!(self.state, TurnState::AssistantSpeaking) {
                    out.push(Self::emit(
                        frame.session_id.clone(),
                        TurnFrame::Interrupted {
                            reason: InterruptReason::VadSpeechStart,
                        },
                    ));
                }
                if !matches!(self.state, TurnState::UserSpeaking) {
                    self.state = TurnState::UserSpeaking;
                    self.speech_started_at = Some(now);
                    out.push(Self::emit(frame.session_id.clone(), TurnFrame::UserStarted));
                }
            }
            Frame::Stt(SttFrame::Final { text, .. }) => {
                if matches!(self.state, TurnState::UserSpeaking | TurnState::Listening) {
                    out.push(Self::emit(
                        frame.session_id.clone(),
                        TurnFrame::UserCommitted { text: text.clone() },
                    ));
                    self.state = TurnState::AssistantSpeaking;
                    self.speech_started_at = None;
                }
            }
            Frame::Turn(TurnFrame::Interrupted { .. }) => {
                self.state = TurnState::Listening;
                self.speech_started_at = None;
            }
            Frame::Turn(TurnFrame::AssistantStarted) => {
                self.state = TurnState::AssistantSpeaking;
            }
            Frame::Turn(TurnFrame::AssistantFinished) => {
                self.state = TurnState::Listening;
            }
            Frame::Lifecycle(crate::frame::LifecycleFrame::SessionEnded)
            | Frame::Lifecycle(crate::frame::LifecycleFrame::Shutdown) => {
                self.state = TurnState::Idle;
                self.speech_started_at = None;
            }
            _ => {}
        }

        if matches!(self.state, TurnState::UserSpeaking)
            && self.speech_duration_exceeds_maximum(now)
        {
            out.push(Self::emit(
                frame.session_id.clone(),
                TurnFrame::Interrupted {
                    reason: InterruptReason::UserCancel,
                },
            ));
            self.state = TurnState::Listening;
            self.speech_started_at = None;
        }

        out.push(frame);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        config::TurnStrategyConfig,
        frame::{Frame, FrameEnvelope, InterruptReason, SessionId, SttFrame, TurnFrame, VadFrame},
        pipeline::FrameProcessor,
    };

    use super::TurnManager;

    #[tokio::test]
    async fn frontend_vad_hint_does_not_start_authoritative_turn() {
        let session_id = SessionId::new();
        let mut manager = TurnManager::new(TurnStrategyConfig::default());
        let frame = FrameEnvelope::new(
            session_id,
            Frame::Vad(VadFrame::FrontendHint {
                speaking: true,
                confidence: Some(0.9),
            }),
        );

        let out = manager.process(frame.clone()).await.unwrap();

        assert_eq!(out, vec![frame]);
    }

    #[tokio::test]
    async fn stt_partial_starts_user_turn_and_final_commits_it() {
        let session_id = SessionId::new();
        let mut manager = TurnManager::new(TurnStrategyConfig::default());

        let partial = FrameEnvelope::new(
            session_id.clone(),
            Frame::Stt(SttFrame::Partial {
                text: "hello".to_string(),
                confidence: Some(0.7),
            }),
        );
        let partial_out = manager.process(partial.clone()).await.unwrap();
        assert!(matches!(
            partial_out[0].frame,
            Frame::Turn(TurnFrame::UserStarted)
        ));
        assert_eq!(partial_out[1], partial);

        let final_frame = FrameEnvelope::new(
            session_id,
            Frame::Stt(SttFrame::Final {
                text: "hello there".to_string(),
                confidence: Some(0.9),
            }),
        );
        let final_out = manager.process(final_frame.clone()).await.unwrap();
        assert!(matches!(
            &final_out[0].frame,
            Frame::Turn(TurnFrame::UserCommitted { text }) if text == "hello there"
        ));
        assert_eq!(final_out[1], final_frame);
    }

    #[tokio::test]
    async fn vad_speech_start_during_assistant_turn_interrupts() {
        let session_id = SessionId::new();
        let mut manager = TurnManager::new(TurnStrategyConfig::default());
        let assistant_started =
            FrameEnvelope::new(session_id.clone(), Frame::Turn(TurnFrame::AssistantStarted));
        manager.process(assistant_started).await.unwrap();

        let speech_started = FrameEnvelope::new(session_id, Frame::Vad(VadFrame::SpeechStarted));
        let out = manager.process(speech_started).await.unwrap();

        assert!(matches!(
            out[0].frame,
            Frame::Turn(TurnFrame::Interrupted {
                reason: InterruptReason::VadSpeechStart
            })
        ));
        assert!(matches!(out[1].frame, Frame::Turn(TurnFrame::UserStarted)));
    }
}
