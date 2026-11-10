//! The Layer B/C driver: prompt, invoke, validate, optionally expand context and repeat.

use crate::model::*;
use crate::prompt::{self, SYSTEM_PROMPT};
use crate::schema::{self, RawAssessment};
use crate::validate;
use concordance_core::evidence::Dependency;
use concordance_core::graph::NodeRef;
use concordance_core::ContentHash;

pub async fn analyze(
    backend: &dyn Backend,
    request: AnalysisRequest,
    provider: &dyn ContextProvider,
    cancel: CancelRx,
) -> Result<AnalysisResult, AnalysisError> {
    let challenge = matches!(request.mode, AnalysisMode::Challenge { .. });
    let schema_value = if challenge {
        schema::challenge_schema()
    } else {
        schema::assess_schema()
    };

    let mut symbols = request.symbols.clone();
    let mut record = InvocationRecord {
        backend: backend.name().to_string(),
        requested_model: request.model.clone(),
        reported_model: None,
        rounds: 0,
        duration_ms: 0,
        cost_usd: None,
        input_tokens: None,
        output_tokens: None,
        session_ids: Vec::new(),
        prompt_hash: ContentHash::of_str(""),
    };

    let mut round = 0u32;
    loop {
        round += 1;
        if *cancel.borrow() {
            return Err(AnalysisError::Cancelled);
        }
        let built = prompt::build(&request, &symbols);
        let resp = backend
            .invoke(
                BackendRequest {
                    system: SYSTEM_PROMPT,
                    prompt: &built.text,
                    schema: &schema_value,
                    model: &request.model,
                },
                cancel.clone(),
            )
            .await?;

        record.rounds = round;
        record.duration_ms += resp.duration_ms;
        record.cost_usd = add_opt(record.cost_usd, resp.cost_usd);
        record.input_tokens = add_opt(record.input_tokens, resp.input_tokens);
        record.output_tokens = add_opt(record.output_tokens, resp.output_tokens);
        if record.reported_model.is_none() {
            record.reported_model = resp.reported_model.clone();
        }
        if let Some(s) = &resp.session_id {
            record.session_ids.push(s.clone());
        }
        record.prompt_hash = ContentHash::of_str(&built.text);

        let raw: RawAssessment = if challenge {
            schema::parse_challenge(&resp.structured)?
        } else {
            schema::parse_assess(&resp.structured)?
        };
        let assessment = validate::finalize(&raw, &built);

        if round < MAX_ROUNDS && !raw.needed_context.is_empty() {
            let mut added = false;
            for want in raw.needed_context.iter().take(MAX_EXPANSIONS_PER_ROUND) {
                let path = want.symbol_path.trim();
                if path.is_empty() {
                    continue;
                }
                if let Some(ctx) = provider.expand(&ContextWanted::Symbol {
                    path: path.to_string(),
                }) {
                    if !symbols.iter().any(|s| s.symbol == ctx.symbol) {
                        symbols.push(ctx);
                        added = true;
                    }
                }
            }
            if added {
                continue;
            }
        }

        let mut dependencies = vec![Dependency {
            node: NodeRef::Claim(request.claim.claim.id.clone()),
            content_hash: request.claim.claim.content_hash.clone(),
        }];
        for &i in &built.included {
            let s = &symbols[i];
            dependencies.push(Dependency {
                node: NodeRef::Symbol(s.symbol.clone()),
                content_hash: s.content_hash.clone(),
            });
        }
        return Ok(AnalysisResult {
            id: request.id,
            assessment,
            invocation: record,
            dependencies,
        });
    }
}

fn add_opt<T: std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x + y),
        (x, None) => x,
        (None, y) => y,
    }
}
