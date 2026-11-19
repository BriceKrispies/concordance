//! Exegra claim results: the statuses that establish `construction` requirements.
//!
//! The input is the JSON `exegra evaluate --json` (or `exegra claims --json`) prints:
//! `{"result": {"results": [{"claimId": "...", "status": "verified"}, ...]}}`. A bare
//! `{"results": [...]}`, a top-level array, and a `{"claims": {"<id>": {"status": ...}}}` map
//! are read too. An entry without a claim id or a status is skipped; a file that is not JSON
//! gives no verdicts (a requirement it would establish stays unverified, with the reason).

use serde_json::Value;
use std::collections::BTreeMap;

/// Claim id -> status (`verified`, `supported`, `violated`, ...), lower case.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return out;
    };
    let list = root
        .pointer("/result/results")
        .or_else(|| root.get("results"))
        .or(Some(&root))
        .and_then(Value::as_array);
    if let Some(items) = list {
        for it in items {
            let id = it
                .get("claimId")
                .or_else(|| it.get("id"))
                .and_then(Value::as_str);
            let status = it.get("status").and_then(Value::as_str);
            if let (Some(id), Some(status)) = (id, status) {
                out.insert(id.to_string(), status.to_ascii_lowercase());
            }
        }
    } else if let Some(map) = root.get("claims").and_then(Value::as_object) {
        for (id, v) in map {
            if let Some(status) = v.get("status").and_then(Value::as_str) {
                out.insert(id.clone(), status.to_ascii_lowercase());
            }
        }
    }
    out
}
