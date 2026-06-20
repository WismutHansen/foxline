use std::collections::BTreeSet;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::{
    frame::{Frame, FrameEnvelope, FrontendToolFrame, SessionId},
    loadout::ToolLoadout,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendToolNegotiation {
    pub required: BTreeSet<String>,
    pub optional: BTreeSet<String>,
    pub advertised: BTreeSet<String>,
    pub effective: BTreeSet<String>,
    pub missing_required: BTreeSet<String>,
}

impl FrontendToolNegotiation {
    pub fn negotiate(loadout: &ToolLoadout, advertised: &[String]) -> Self {
        let required = loadout
            .required_frontend
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let optional = loadout
            .optional_frontend
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let advertised = advertised.iter().cloned().collect::<BTreeSet<_>>();
        let allowed = required.union(&optional).cloned().collect::<BTreeSet<_>>();
        let effective = allowed
            .intersection(&advertised)
            .cloned()
            .collect::<BTreeSet<_>>();
        let missing_required = required
            .difference(&advertised)
            .cloned()
            .collect::<BTreeSet<_>>();

        Self {
            required,
            optional,
            advertised,
            effective,
            missing_required,
        }
    }

    pub fn ensure_startup_allowed(&self) -> Result<()> {
        if !self.missing_required.is_empty() {
            bail!(
                "missing required frontend tools: {}",
                self.missing_required
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        Ok(())
    }

    pub fn is_authorized(&self, name: &str) -> bool {
        self.effective.contains(name)
    }

    pub fn negotiated_frame(&self, session_id: SessionId) -> FrameEnvelope {
        FrameEnvelope::new(
            session_id,
            Frame::FrontendTool(FrontendToolFrame::Negotiated {
                tools: self.effective.iter().cloned().collect(),
            }),
        )
    }
}

#[derive(Debug, Clone)]
pub struct FrontendToolRouter {
    negotiation: FrontendToolNegotiation,
}

impl FrontendToolRouter {
    pub fn new(negotiation: FrontendToolNegotiation) -> Self {
        Self { negotiation }
    }

    pub fn route_call(
        &self,
        session_id: SessionId,
        name: String,
        arguments: Value,
    ) -> FrameEnvelope {
        if self.negotiation.is_authorized(&name) {
            FrameEnvelope::new(
                session_id,
                Frame::FrontendTool(FrontendToolFrame::Call { name, arguments }),
            )
        } else {
            FrameEnvelope::new(
                session_id,
                Frame::FrontendTool(FrontendToolFrame::Rejected {
                    name,
                    reason: "frontend tool is not negotiated for this session".to_string(),
                }),
            )
        }
    }

    pub fn result_frame(
        &self,
        session_id: SessionId,
        call_id: String,
        result: Value,
    ) -> FrameEnvelope {
        FrameEnvelope::new(
            session_id,
            Frame::FrontendTool(FrontendToolFrame::Result { call_id, result }),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        frame::{Frame, FrontendToolFrame, SessionId},
        loadout::ToolLoadout,
    };

    use super::{FrontendToolNegotiation, FrontendToolRouter};

    #[test]
    fn effective_tools_are_allowed_intersect_advertised() {
        let loadout = ToolLoadout {
            required_frontend: vec!["codec.display".to_string()],
            optional_frontend: vec!["codec.avatar".to_string(), "codec.map".to_string()],
            allowed_pi: Vec::new(),
        };

        let negotiated = FrontendToolNegotiation::negotiate(
            &loadout,
            &[
                "codec.display".to_string(),
                "codec.map".to_string(),
                "extra".to_string(),
            ],
        );

        assert!(negotiated.ensure_startup_allowed().is_ok());
        assert_eq!(
            negotiated.effective.iter().cloned().collect::<Vec<_>>(),
            ["codec.display".to_string(), "codec.map".to_string()]
        );
    }

    #[test]
    fn missing_required_tools_fail_startup() {
        let loadout = ToolLoadout {
            required_frontend: vec!["codec.display".to_string()],
            optional_frontend: Vec::new(),
            allowed_pi: Vec::new(),
        };

        let negotiated = FrontendToolNegotiation::negotiate(&loadout, &[]);

        assert!(negotiated.ensure_startup_allowed().is_err());
    }

    #[test]
    fn router_rejects_unauthorized_calls() {
        let loadout = ToolLoadout {
            required_frontend: Vec::new(),
            optional_frontend: vec!["codec.display".to_string()],
            allowed_pi: Vec::new(),
        };
        let negotiated =
            FrontendToolNegotiation::negotiate(&loadout, &["codec.display".to_string()]);
        let router = FrontendToolRouter::new(negotiated);

        let ok = router.route_call(SessionId::new(), "codec.display".to_string(), json!({}));
        let rejected = router.route_call(SessionId::new(), "extra".to_string(), json!({}));

        assert!(matches!(
            ok.frame,
            Frame::FrontendTool(FrontendToolFrame::Call { .. })
        ));
        assert!(matches!(
            rejected.frame,
            Frame::FrontendTool(FrontendToolFrame::Rejected { .. })
        ));
    }
}
