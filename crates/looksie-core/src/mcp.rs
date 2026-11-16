//! `looksie --mcp` (and `looksie-cli mcp`): lets the reviewer's own AI agent (Claude Code, Codex) use Looksie.
//!
//! This is a small Model Context Protocol server over standard input and output: one JSON-RPC message per line.
//! Nothing is posted to any pull request. The only thing it writes is an AI answer, into Looksie's own folder,
//! so the app can show it: that is how someone uses their own agent for Looksie's AI layer.

use crate::{ai_store, load_review, parse_pr};
use looksie_connectors::{github::GitHub, Connector};
use looksie_protocol::{ClaimStatus, Flag, Review, Tier};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// What the agent is asked to do with the `review_pull_request` prompt (a slash command in Claude Code).
const AGENT_TASK: &str = "Use the looksie tools. Call ai_prompt for these pull requests, follow its instructions exactly to write the JSON answer, then call save_ai_layer with that answer. Then tell me in three short lines what you found and say the answer is now in the Looksie app. Do not approve or reject anything.";

fn tools(can_save: bool) -> Value {
    let prs = json!({ "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "One or more pull requests, each as owner/repo#123 or a pull request URL. Several are reviewed together as one group." });
    let mut t = json!([
        {
            "name": "review_queue",
            "description": "List the open pull requests waiting on the signed-in user's review, and the ones they or their agent opened. Use this first when asked what needs reviewing.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "review",
            "description": "Build Looksie's review of one or more pull requests: what the authors claim (checked against the code and CI), what needs a human, the decisions a reviewer has to make with the reasons each was picked, the routine changes, and what was skipped. Facts and rules only, no AI. Use step ids from this with step_changes.",
            "inputSchema": { "type": "object", "properties": { "pull_requests": prs }, "required": ["pull_requests"] }
        },
        {
            "name": "step_changes",
            "description": "The code changes for one step of a review, so a decision can be judged against the actual diff.",
            "inputSchema": { "type": "object", "properties": { "pull_requests": prs, "step_id": { "type": "string", "description": "A step id from the review tool" } }, "required": ["pull_requests", "step_id"] }
        }
    ]);
    if can_save {
        let list = t.as_array_mut().expect("a list");
        list.push(json!({
            "name": "ai_prompt",
            "description": "Get Looksie's AI task for one or more pull requests: the decisions, their changes, and the exact JSON answer Looksie wants. Write that answer yourself, then pass it to save_ai_layer. Every piece of evidence must be a line copied exactly from the changes, or Looksie throws that part away.",
            "inputSchema": { "type": "object", "properties": { "pull_requests": prs }, "required": ["pull_requests"] }
        }));
        list.push(json!({
            "name": "save_ai_layer",
            "description": "Save your answer to ai_prompt into Looksie, so the reviewer sees it in the Looksie app (summary, a question per decision, claim checks). Looksie checks every piece of evidence against the real changes and keeps only what matches.",
            "inputSchema": { "type": "object", "properties": {
                "pull_requests": prs,
                "answer": { "type": "object", "description": "The JSON object ai_prompt asked for: summary, steps, claims." },
                "from": { "type": "string", "description": "Which agent wrote it, shown in the app, for example \"Claude Code\" or \"Codex\"." }
            }, "required": ["pull_requests", "answer"] }
        }));
    }
    t
}

fn prompts() -> Value {
    json!([{
        "name": "review_pull_request",
        "description": "Write Looksie's AI layer for a pull request (or several, together) with this agent, and save it into the Looksie app.",
        "arguments": [{ "name": "pull_requests", "description": "owner/repo#123, or several separated by spaces", "required": true }]
    }])
}

fn review_text(r: &Review) -> String {
    let mut t = format!("# {}\n", r.briefing.title);
    for p in &r.prs {
        let s = &p.summary;
        let ci = if p.ci.total == 0 { "no CI run on this commit".to_string() } else if p.ci.failed > 0 { format!("{} of {} CI checks failed", p.ci.failed, p.ci.total) } else { format!("{} CI checks, none failed", p.ci.total) };
        t += &format!("- {} by {}{}: +{} -{} in {} files, {}, into {}. {}\n", s.pr.label(), s.author, if s.agent { " (agent-written)" } else { "" }, s.additions, s.deletions, s.changed_files, ci, p.base_ref, s.url);
    }
    t += "\n## What the authors say\n";
    for d in &r.briefing.descriptions {
        let short: String = d.text.chars().take(900).collect();
        t += &format!("### {}\n{short}\n\n", d.pr.short());
    }
    if !r.briefing.claims.is_empty() {
        t += "## Claims, checked\n";
        for c in &r.briefing.claims {
            let mark = match c.status { ClaimStatus::Backed => "BACKED", ClaimStatus::Contradicted => "CONTRADICTED", ClaimStatus::NotFound => "NO EVIDENCE", ClaimStatus::Unchecked => "NOT CHECKED" };
            t += &format!("- [{mark}] {} ({})\n", c.text, c.note);
        }
        t += "\n";
    }
    for (title, list) in [("Needs a human", &r.briefing.needs_you), ("Not mentioned in the description", &r.briefing.not_mentioned), ("How the pull requests fit together", &r.briefing.fits)] {
        if !list.is_empty() {
            t += &format!("## {title}\n");
            for f in list {
                t += &format!("- {}\n", f.text);
            }
            t += "\n";
        }
    }
    t += "## Decisions (need a reviewer's judgement)\n";
    for (i, s) in r.steps.iter().filter(|s| s.tier == Tier::Decide).enumerate() {
        let flag = match s.flag { Flag::Problem => " [LIKELY PROBLEM]", Flag::Look => " [look closely]", Flag::None => "" };
        t += &format!("{}. {}{flag}\n   step id: {}\n   why it needs a decision: {}\n   facts: {}\n", i + 1, s.title, s.id, s.reasons.join("; "), s.facts.iter().map(|f| f.text.as_str()).collect::<Vec<_>>().join("; "));
        if let Some(q) = &s.question {
            t += &format!("   a rule asks: {q}\n");
        }
        t += &format!("   files: {}\n", s.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(", "));
    }
    let routine: Vec<_> = r.steps.iter().filter(|s| s.tier == Tier::Glance).collect();
    if !routine.is_empty() {
        t += "\n## Quick looks (probably need no decision)\n";
        for s in routine {
            t += &format!("- {} (step id: {}; {}; {} files)\n", s.title, s.id, s.reasons.join("; "), s.files.len());
        }
    }
    if !r.skipped.is_empty() {
        t += &format!("\n## Skipped\n{} files, {} lines (generated, vendored, lockfile churn or pure renames).\n", r.skipped.len(), r.skipped.iter().map(|s| s.lines).sum::<u32>());
    }
    t
}

fn refs_key(args: &Value) -> String {
    args.get("pull_requests").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+")).unwrap_or_default()
}

struct Server {
    gh: GitHub,
    cache: HashMap<String, Review>,
    /// Where AI answers are saved for the app. None for the command-line tool, which has no app to show them.
    ai_dir: Option<PathBuf>,
    /// Private repositories need Looksie Pro (or the trial), as in the app.
    private_ok: bool,
}

impl Server {
    async fn review_for(&mut self, args: &Value) -> Result<&Review, String> {
        let refs: Vec<String> = args.get("pull_requests").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if refs.is_empty() {
            return Err("Give at least one pull request, as owner/repo#123.".into());
        }
        let key = refs_key(args);
        if !self.cache.contains_key(&key) {
            let prs = refs.iter().map(|r| parse_pr(r)).collect::<Result<Vec<_>, _>>()?;
            let review = load_review(&self.gh, &prs, None).await?;
            if !self.private_ok {
                if let Some(p) = review.prs.iter().find(|p| p.summary.private) {
                    return Err(format!("{} is in a private repository, which needs Looksie Pro. Start the free trial or add your licence key in the Looksie app.", p.summary.pr.label()));
                }
            }
            self.cache.insert(key.clone(), review);
        }
        Ok(&self.cache[&key])
    }

    async fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "review_queue" => {
                let prs = self.gh.list_my_prs().await?;
                if prs.is_empty() {
                    return Ok("Nothing is waiting on you.".into());
                }
                Ok(prs.iter().map(|p| format!("- {} · {} · by {}{} · +{} -{} · {}{}", p.pr.label().replace(" #", "#"), p.title, p.author, if p.agent { " (agent-written)" } else { "" }, p.additions, p.deletions, if p.relation == "mine" { "opened by you" } else { "your review is requested" }, if p.private { " · private" } else { "" })).collect::<Vec<_>>().join("\n"))
            }
            "review" => Ok(review_text(self.review_for(args).await?)),
            "ai_prompt" => {
                // Always the latest commits: an agent session can stay open while new commits arrive.
                self.cache.remove(&refs_key(args));
                let review = self.review_for(args).await?;
                Ok(format!("{}\n\nWhen your JSON answer is ready, call save_ai_layer with the same pull_requests and the answer as an object.", looksie_engine::ai::prompt(review)))
            }
            "save_ai_layer" => {
                let dir = self.ai_dir.clone().ok_or("Saving needs the Looksie app's MCP server (looksie --mcp).")?;
                let answer = match args.get("answer") {
                    Some(Value::String(text)) => crate::extract_json(text)?,
                    Some(v) if v.is_object() => v.clone(),
                    _ => return Err("answer must be the JSON object ai_prompt asked for.".into()),
                };
                let from: String = args.get("from").and_then(Value::as_str).unwrap_or("your AI agent").trim().chars().take(40).collect();
                let review = self.review_for(args).await?;
                let layer = looksie_engine::ai::read(review, &answer, if from.is_empty() { "your AI agent" } else { &from });
                ai_store::save(&dir, review, &layer)?;
                let decisions = review.steps.iter().filter(|s| s.tier == Tier::Decide).count();
                let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
                Ok(format!(
                    "Saved into Looksie: {}{} of {} kept, and {}{}. Open the review in the Looksie app to see them.",
                    if layer.summary.is_some() { "a summary, " } else { "" },
                    n(layer.steps.len(), "decision question", "decision questions"),
                    decisions,
                    n(layer.claims.len(), "claim check", "claim checks"),
                    if layer.dropped > 0 {
                        format!(". {} thrown away: evidence that is not a real line from the changes, a step that is not in this review, or a repeat", n(layer.dropped as usize, "entry was", "entries were"))
                    } else {
                        String::new()
                    }
                ))
            }
            "step_changes" => {
                let id = args.get("step_id").and_then(Value::as_str).unwrap_or("").to_string();
                let review = self.review_for(args).await?;
                let step = review.steps.iter().find(|s| s.id == id).ok_or_else(|| format!("No step with id '{id}'. Call review to see the step ids."))?;
                Ok(format!("{}\n\n{}", step.title, looksie_engine::ai::diff_text(review, step, 1200)))
            }
            other => Err(format!("Unknown tool: {other}")),
        }
    }
}

/// Serves until standard input closes. `ai_dir` is where AI answers go for the app; None leaves the AI tools out.
pub async fn serve(gh: GitHub, ai_dir: Option<PathBuf>, private_ok: bool) -> Result<(), String> {
    let can_save = ai_dir.is_some();
    let mut server = Server { gh, cache: HashMap::new(), ai_dir, private_ok };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no reply
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"),
                "capabilities": if can_save { json!({ "tools": {}, "prompts": {} }) } else { json!({ "tools": {} }) },
                "serverInfo": { "name": "looksie", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Looksie turns pull requests into a short list of decisions for a human reviewer. Present decisions to the user; never approve or reject on their behalf. To add AI help to a review in the Looksie app: call ai_prompt, write the JSON it asks for, then call save_ai_layer."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools(can_save) })),
            "prompts/list" if can_save => Ok(json!({ "prompts": prompts() })),
            "prompts/get" if can_save => {
                let refs = params.pointer("/arguments/pull_requests").and_then(Value::as_str).unwrap_or("").trim().to_string();
                if refs.is_empty() {
                    Err((-32602, "Give the pull requests, as owner/repo#123.".to_string()))
                } else {
                    Ok(json!({
                        "description": "Looksie's AI layer for these pull requests",
                        "messages": [{ "role": "user", "content": { "type": "text", "text": format!("Pull requests: {refs}\n\n{AGENT_TASK}") } }]
                    }))
                }
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                Ok(match server.call(name, &args).await {
                    Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                })
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        let reply = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    Ok(())
}
