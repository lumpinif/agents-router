use crate::agent_controller::{
    AgentControllerAdapter, AgentControllerError, AgentControllerErrorKind, AgentControllerFuture,
    AgentControllerRequest,
};
use crate::agent_integration_catalog::AgentControllerKind;

#[derive(Debug, Default)]
pub struct CodexAppServerController;

impl CodexAppServerController {
    pub fn new() -> Self {
        Self
    }
}

impl AgentControllerAdapter for CodexAppServerController {
    fn controller_kind(&self) -> AgentControllerKind {
        AgentControllerKind::CodexAppServer
    }

    fn continue_session<'a>(
        &'a self,
        request: AgentControllerRequest,
    ) -> AgentControllerFuture<'a> {
        Box::pin(async move {
            Err(AgentControllerError::from_request(
                &request,
                AgentControllerErrorKind::ControllerUnavailable,
                "Codex App Server continuation is not available in this build",
            ))
        })
    }
}
