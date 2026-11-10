//! A scripted backend for tests (also usable from other crates' integration tests).

use crate::model::*;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Returns pre-programmed structured responses in order; records the prompts it saw.
pub struct ScriptedBackend {
    responses: Mutex<VecDeque<Result<serde_json::Value, AnalysisError>>>,
    prompts: Mutex<Vec<String>>,
}

impl ScriptedBackend {
    pub fn new(responses: Vec<serde_json::Value>) -> Self {
        Self::with_results(responses.into_iter().map(Ok).collect())
    }

    pub fn with_results(responses: Vec<Result<serde_json::Value, AnalysisError>>) -> Self {
        ScriptedBackend {
            responses: Mutex::new(responses.into()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    /// Prompts received so far, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

impl Backend for ScriptedBackend {
    fn name(&self) -> &str {
        "scripted"
    }

    fn invoke<'a>(
        &'a self,
        req: BackendRequest<'a>,
        _cancel: CancelRx,
    ) -> BoxFuture<'a, Result<BackendResponse, AnalysisError>> {
        if let Ok(mut p) = self.prompts.lock() {
            p.push(req.prompt.to_string());
        }
        let next = self.responses.lock().ok().and_then(|mut q| q.pop_front());
        Box::pin(async move {
            match next {
                None => Err(AnalysisError::Invocation(
                    "scripted backend exhausted".into(),
                )),
                Some(Err(e)) => Err(e),
                Some(Ok(structured)) => Ok(BackendResponse {
                    structured,
                    reported_model: Some("scripted-model".into()),
                    duration_ms: 1,
                    cost_usd: Some(0.0),
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    session_id: Some("scripted-session".into()),
                }),
            }
        })
    }
}
