//! How Looksie asks an AI model about a review, with the reviewer's own API key.
//!
//! What this crate does, so you can check it:
//! - It sends one request, either to Anthropic's Messages API (api.anthropic.com) or to OpenAI's Responses API
//!   (api.openai.com), with the key the reviewer pasted in for that company.
//! - The request is the prompt the app can show before anything is sent, and a JSON schema the answer must follow.
//! - The model gets no tools: it cannot read files, run commands or browse. It only answers.
//! - Nothing else is sent, and each key goes to no one but its own company. OpenAI is asked not to store the answer.
//!
//! Looksie does not use anyone's Claude.ai or ChatGPT sign-in. People who would rather use their own Claude Code or
//! Codex do it the other way round: their agent calls Looksie as a tool (see `looksie --mcp`).

use serde_json::{json, Value};
use std::time::Duration;

const ANTHROPIC_API: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2026-10-21";
/// Retries a declined request on a suitable model inside the same call. Only for models that support it.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const OPENAI_API: &str = "https://api.openai.com";

/// Room for the answer and the model's thinking.
const MAX_OUTPUT: u32 = 16_000;

/// The company a key and a model belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
}

pub const PROVIDERS: &[Provider] = &[Provider::Anthropic, Provider::OpenAi];

impl Provider {
    /// Stable id, used for the keychain entry and by the app.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
        }
    }
    /// What the button says: "Ask Claude", "Ask GPT".
    pub fn assistant(self) -> &'static str {
        match self {
            Provider::Anthropic => "Claude",
            Provider::OpenAi => "GPT",
        }
    }
    /// What a key from this company looks like, for the placeholder.
    pub fn key_start(self) -> &'static str {
        match self {
            Provider::Anthropic => "sk-ant-",
            Provider::OpenAi => "sk-",
        }
    }
    /// Where to make a key.
    pub fn keys_url(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://platform.claude.com/settings/keys",
            Provider::OpenAi => "https://platform.openai.com/api-keys",
        }
    }
    pub fn keys_page(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic's Console",
            Provider::OpenAi => "OpenAI's dashboard",
        }
    }
    pub fn from_id(id: &str) -> Option<Provider> {
        PROVIDERS.iter().copied().find(|p| p.id() == id)
    }
    /// Which company a pasted key is for, from how it starts. Anthropic keys start sk-ant-, OpenAI keys sk-.
    pub fn of_key(key: &str) -> Option<Provider> {
        let k = key.trim();
        if k.starts_with("sk-ant-") {
            Some(Provider::Anthropic)
        } else if k.starts_with("sk-") {
            Some(Provider::OpenAi)
        } else {
            None
        }
    }
}

/// The models a reviewer can pick. The first is the default, and the first of each company is that company's default.
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    pub provider: Provider,
    /// US dollars per million input and output tokens, for the rough cost shown before asking.
    pub price_in: f64,
    pub price_out: f64,
    fallback: bool,
}

pub const MODELS: &[Model] = &[
    Model { id: "claude-opus-5-5", name: "Claude Opus 5.5", note: "Most careful", provider: Provider::Anthropic, price_in: 4.0, price_out: 20.0, fallback: true },
    Model { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", note: "Half the price, nearly as good", provider: Provider::Anthropic, price_in: 2.0, price_out: 10.0, fallback: true },
    Model { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", note: "Cheapest and quickest", provider: Provider::Anthropic, price_in: 0.10, price_out: 0.50, fallback: false },
    Model { id: "gpt-6.1-sol", name: "GPT-6.1 Sol", note: "Careful, at a fair price", provider: Provider::OpenAi, price_in: 2.0, price_out: 10.0, fallback: false },
    Model { id: "gpt-6-astra", name: "GPT-6 Astra", note: "OpenAI's most capable, five times the price", provider: Provider::OpenAi, price_in: 10.0, price_out: 50.0, fallback: false },
    Model { id: "gpt-6-luna", name: "GPT-6 Luna", note: "Cheapest and quickest", provider: Provider::OpenAi, price_in: 0.10, price_out: 0.50, fallback: false },
];

pub fn model(id: &str) -> &'static Model {
    MODELS.iter().find(|m| m.id == id).unwrap_or(&MODELS[0])
}

/// The first, recommended model of a company.
pub fn default_for(p: Provider) -> &'static Model {
    MODELS.iter().find(|m| m.provider == p).unwrap_or(&MODELS[0])
}

/// A rough cost in US dollars for a prompt of this many characters (about four characters a token, and a few
/// thousand tokens of answer and thinking).
pub fn rough_cost(model_id: &str, prompt_chars: usize) -> f64 {
    let m = model(model_id);
    let input = prompt_chars as f64 / 4.0;
    let output = 6_000.0;
    (input * m.price_in + output * m.price_out) / 1_000_000.0
}

fn client(timeout: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(Duration::from_secs(timeout)).build().map_err(|e| e.to_string())
}

/// What an API error means, in words a reviewer can act on. Both companies put the detail in error.message.
fn explain(p: Provider, status: u16, body: &str) -> String {
    let who = p.name();
    let detail: String = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    let why = match status {
        401 => format!("{who} did not accept the API key. Check it in Settings, Your AI."),
        403 => "This API key is not allowed to use that model.".to_string(),
        404 => format!("{who} does not know that model. Pick another in Settings, Your AI."),
        413 => "This review is too large to send in one go.".to_string(),
        429 => format!("{who}'s rate limit or spending limit was reached. Try again in a minute, or check your billing with {who}."),
        500..=599 => format!("{who} had a problem. Try again in a minute."),
        _ => format!("{who} answered {status}."),
    };
    if detail.is_empty() || status == 401 { why } else { format!("{why} ({detail})") }
}

/// Checks a key without spending anything: lists the models it can use.
pub async fn check_key(p: Provider, key: &str) -> Result<(), String> {
    let req = match p {
        Provider::Anthropic => client(20)?.get(format!("{ANTHROPIC_API}/v1/models?limit=1")).header("x-api-key", key.trim()).header("anthropic-version", ANTHROPIC_VERSION),
        Provider::OpenAi => client(20)?.get(format!("{OPENAI_API}/v1/models")).bearer_auth(key.trim()),
    };
    let res = req.send().await.map_err(|_| format!("Could not reach {}. Check your internet connection.", p.name()))?;
    let status = res.status().as_u16();
    if status == 200 {
        return Ok(());
    }
    Err(explain(p, status, &res.text().await.unwrap_or_default()))
}

/// The shape the answer must have. Every object lists all its fields and allows no others, which both companies'
/// strict structured output needs.
pub fn answer_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "steps", "claims"],
        "properties": {
            "summary": { "type": "string" },
            "steps": { "type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "question", "why", "flag", "evidence"],
                "properties": {
                    "id": { "type": "string" },
                    "question": { "type": "string" },
                    "why": { "type": "string" },
                    "flag": { "type": "string", "enum": ["none", "look", "problem"] },
                    "evidence": { "type": "string" }
                }
            }},
            "claims": { "type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["index", "status", "note", "evidence"],
                "properties": {
                    "index": { "type": "integer" },
                    "status": { "type": "string", "enum": ["backed", "contradicted"] },
                    "note": { "type": "string" },
                    "evidence": { "type": "string" }
                }
            }}
        }
    })
}

/// The request body sent for this model. Kept separate so it can be shown and tested.
pub fn request_body(model_id: &str, prompt: &str) -> Value {
    let m = model(model_id);
    match m.provider {
        Provider::Anthropic => {
            let mut body = json!({
                "model": m.id,
                "max_tokens": MAX_OUTPUT,
                "output_config": { "effort": "medium", "format": { "type": "json_schema", "schema": answer_schema() } },
                "messages": [{ "role": "user", "content": prompt }]
            });
            if m.fallback {
                body["fallbacks"] = json!("default");
            }
            body
        }
        Provider::OpenAi => json!({
            "model": m.id,
            "input": [{ "role": "user", "content": prompt }],
            "text": { "format": { "type": "json_schema", "name": "looksie_review", "strict": true, "schema": answer_schema() } },
            "reasoning": { "effort": "medium" },
            "max_output_tokens": MAX_OUTPUT,
            // Not kept on OpenAI's side for later retrieval. Looksie keeps the answer itself.
            "store": false
        }),
    }
}

/// Pulls the answer out of an Anthropic Messages API response.
pub fn read_response(v: &Value) -> Result<Value, String> {
    match v.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => return Err("The model declined to answer this one.".into()),
        Some("max_tokens") => return Err("The answer was cut off: this review is too large for one answer.".into()),
        _ => {}
    }
    let text: String = v
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    extract_json(&text)
}

/// Pulls the answer out of an OpenAI Responses API response.
pub fn read_openai(v: &Value) -> Result<Value, String> {
    match v.get("status").and_then(Value::as_str) {
        Some("incomplete") => {
            return Err(match v.pointer("/incomplete_details/reason").and_then(Value::as_str) {
                Some("content_filter") => "The model declined to answer this one.".into(),
                _ => "The answer was cut off: this review is too large for one answer.".into(),
            })
        }
        Some("failed") => {
            let why = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("no reason given");
            return Err(format!("OpenAI could not answer: {}", why.chars().take(300).collect::<String>()));
        }
        _ => {}
    }
    let parts: Vec<&Value> = v
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| o.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|o| o.get("content").and_then(Value::as_array))
        .flatten()
        .collect();
    let text: String = parts.iter().filter(|c| c.get("type").and_then(Value::as_str) == Some("output_text")).filter_map(|c| c.get("text").and_then(Value::as_str)).collect();
    if text.trim().is_empty() && parts.iter().any(|c| c.get("type").and_then(Value::as_str) == Some("refusal")) {
        return Err("The model declined to answer this one.".into());
    }
    extract_json(&text)
}

/// Sends one prompt and returns the JSON answer. Waits up to ten minutes.
pub async fn ask(key: &str, model_id: &str, prompt: &str) -> Result<Value, String> {
    let m = model(model_id);
    let p = m.provider;
    let http = client(600)?;
    let req = match p {
        Provider::Anthropic => {
            let r = http.post(format!("{ANTHROPIC_API}/v1/messages")).header("x-api-key", key.trim()).header("anthropic-version", ANTHROPIC_VERSION);
            if m.fallback { r.header("anthropic-beta", FALLBACK_BETA) } else { r }
        }
        Provider::OpenAi => http.post(format!("{OPENAI_API}/v1/responses")).bearer_auth(key.trim()),
    };
    let res = req.json(&request_body(model_id, prompt)).send().await.map_err(|e| {
        if e.is_timeout() { format!("{} took more than ten minutes. Try a smaller review or a quicker model.", p.name()) } else { format!("Could not reach {}. Check your internet connection.", p.name()) }
    })?;
    let status = res.status().as_u16();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(explain(p, status, &body));
    }
    let v: Value = serde_json::from_str(&body).map_err(|_| format!("{} sent something unexpected.", p.name()))?;
    match p {
        Provider::Anthropic => read_response(&v),
        Provider::OpenAi => read_openai(&v),
    }
}

/// Pulls the first JSON object out of a model's reply, whether or not it wrapped it in a code fence.
pub fn extract_json(text: &str) -> Result<Value, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { return Err("The AI did not answer with JSON.".into()) };
    serde_json::from_str(&text[a..=b]).map_err(|e| format!("The AI's answer was not valid JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_inside_a_reply() {
        assert_eq!(extract_json("Here you go:\n```json\n{\"a\": 1}\n```").unwrap()["a"], 1);
        assert!(extract_json("no json here").is_err());
    }

    #[test]
    fn the_request_has_no_tools_and_a_strict_schema() {
        let b = request_body("claude-opus-5-5", "hello");
        assert!(b.get("tools").is_none(), "the model gets no tools");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["output_config"]["format"]["schema"]["additionalProperties"], false);
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["messages"][0]["content"], "hello");
    }

    #[test]
    fn the_openai_request_has_no_tools_a_strict_schema_and_is_not_stored() {
        let b = request_body("gpt-6.1-sol", "hello");
        assert!(b.get("tools").is_none(), "the model gets no tools");
        assert_eq!(b["text"]["format"]["type"], "json_schema");
        assert_eq!(b["text"]["format"]["strict"], true);
        assert_eq!(b["text"]["format"]["schema"]["additionalProperties"], false);
        assert_eq!(b["store"], false);
        assert_eq!(b["input"][0]["content"], "hello");
        assert!(b.get("fallbacks").is_none() && b.get("messages").is_none());
    }

    #[test]
    fn haiku_gets_no_fallback_and_unknown_models_fall_back_to_the_default() {
        assert!(request_body("claude-haiku-5-5", "x").get("fallbacks").is_none());
        assert_eq!(model("made-up").id, "claude-opus-5-5");
    }

    #[test]
    fn keys_are_told_apart_by_how_they_start() {
        assert_eq!(Provider::of_key(" sk-ant-api03-abc "), Some(Provider::Anthropic));
        assert_eq!(Provider::of_key("sk-proj-abc"), Some(Provider::OpenAi));
        assert_eq!(Provider::of_key("ghp_abc"), None);
        assert_eq!(default_for(Provider::OpenAi).id, "gpt-6.1-sol");
        assert_eq!(default_for(Provider::Anthropic).id, "claude-opus-5-5");
        for p in PROVIDERS {
            assert_eq!(Provider::from_id(p.id()), Some(*p));
        }
    }

    #[test]
    fn every_schema_object_forbids_extra_fields() {
        fn walk(v: &Value) {
            if v.get("type") == Some(&json!("object")) {
                assert_eq!(v["additionalProperties"], false);
                let req: Vec<&str> = v["required"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
                for k in v["properties"].as_object().unwrap().keys() {
                    assert!(req.contains(&k.as_str()), "{k} must be required");
                }
            }
            if let Some(o) = v.as_object() {
                o.values().for_each(walk);
            }
        }
        walk(&answer_schema());
    }

    #[test]
    fn reads_text_blocks_and_refusals() {
        let ok = json!({ "stop_reason": "end_turn", "content": [{ "type": "thinking", "thinking": "" }, { "type": "text", "text": "{\"summary\": \"s\", \"steps\": [], \"claims\": []}" }] });
        assert_eq!(read_response(&ok).unwrap()["summary"], "s");
        assert!(read_response(&json!({ "stop_reason": "refusal", "content": [] })).is_err());
        assert!(read_response(&json!({ "stop_reason": "max_tokens", "content": [] })).is_err());
    }

    #[test]
    fn reads_openai_output_text_refusals_and_cut_offs() {
        let ok = json!({ "status": "completed", "output": [
            { "type": "reasoning", "summary": [] },
            { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "{\"summary\": \"s\", \"steps\": [], \"claims\": []}", "annotations": [] }] }
        ]});
        assert_eq!(read_openai(&ok).unwrap()["summary"], "s");
        let refused = json!({ "status": "completed", "output": [{ "type": "message", "content": [{ "type": "refusal", "refusal": "no" }] }] });
        assert!(read_openai(&refused).unwrap_err().contains("declined"));
        let cut = json!({ "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" }, "output": [] });
        assert!(read_openai(&cut).unwrap_err().contains("cut off"));
        let failed = json!({ "status": "failed", "error": { "message": "boom" }, "output": [] });
        assert!(read_openai(&failed).unwrap_err().contains("boom"));
    }

    #[test]
    fn errors_are_plain_words() {
        assert!(explain(Provider::Anthropic, 401, "{}").contains("API key"));
        assert!(explain(Provider::OpenAi, 401, "{}").starts_with("OpenAI"));
        assert!(explain(Provider::Anthropic, 429, "{\"error\":{\"message\":\"slow down\"}}").contains("slow down"));
    }

    #[test]
    fn rough_cost_is_cheaper_on_cheaper_models() {
        assert!(rough_cost("claude-haiku-5-5", 40_000) < rough_cost("claude-sonnet-5-5", 40_000));
        assert!(rough_cost("claude-sonnet-5-5", 40_000) < rough_cost("claude-opus-5-5", 40_000));
        assert!(rough_cost("gpt-6-luna", 40_000) < rough_cost("gpt-6.1-sol", 40_000));
        assert!(rough_cost("gpt-6.1-sol", 40_000) < rough_cost("gpt-6-astra", 40_000));
    }
}
