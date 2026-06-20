use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::frame::AvatarActionFrame;

#[derive(Debug, Clone, Default)]
pub struct AvatarActionRouter {
    supported: BTreeSet<String>,
}

impl AvatarActionRouter {
    pub fn new<I, S>(supported: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            supported: supported.into_iter().map(Into::into).collect(),
        }
    }

    pub fn route(&self, action: &AvatarActionFrame) -> Option<Value> {
        let action_name = action_name(action);
        if !self.supported.is_empty() && !self.supported.contains(action_name) {
            return None;
        }

        Some(match action {
            AvatarActionFrame::SetState { state } => {
                json!({ "type": "set_state", "state": state })
            }
            AvatarActionFrame::SetExpression { expression } => {
                json!({ "type": "set_expression", "expression": expression })
            }
            AvatarActionFrame::Focus { target } => {
                json!({ "type": "focus", "target": target })
            }
            AvatarActionFrame::PlayAnimation { name } => {
                json!({ "type": "play_animation", "name": name })
            }
            AvatarActionFrame::Clear => json!({ "type": "clear" }),
        })
    }
}

fn action_name(action: &AvatarActionFrame) -> &'static str {
    match action {
        AvatarActionFrame::SetState { .. } => "set_state",
        AvatarActionFrame::SetExpression { .. } => "set_expression",
        AvatarActionFrame::Focus { .. } => "focus",
        AvatarActionFrame::PlayAnimation { .. } => "play_animation",
        AvatarActionFrame::Clear => "clear",
    }
}

#[cfg(test)]
mod tests {
    use crate::frame::AvatarActionFrame;

    use super::AvatarActionRouter;

    #[test]
    fn routes_supported_semantic_action() {
        let router = AvatarActionRouter::new(["set_expression"]);

        let event = router
            .route(&AvatarActionFrame::SetExpression {
                expression: "alert".to_string(),
            })
            .unwrap();

        assert_eq!(event["type"], "set_expression");
        assert_eq!(event["expression"], "alert");
    }

    #[test]
    fn rejects_unadvertised_action() {
        let router = AvatarActionRouter::new(["focus"]);

        assert!(router
            .route(&AvatarActionFrame::PlayAnimation {
                name: "speaking".to_string()
            })
            .is_none());
    }

    #[test]
    fn empty_capability_set_allows_actions_for_development() {
        let router = AvatarActionRouter::default();

        assert!(router.route(&AvatarActionFrame::Clear).is_some());
    }
}
