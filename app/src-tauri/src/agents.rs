//! Connecting the reviewer's own Claude Code or Codex to Looksie, in one click.
//!
//! Both get the same Looksie skill (the open Agent Skills format both read) and Looksie's MCP server (`looksie --mcp`).
//! - Claude Code: Looksie writes a small plugin (the skill plus the MCP server, with this computer's real path to the
//!   app filled in) inside a local marketplace in Looksie's own folder, then runs `claude plugin marketplace add` and
//!   `claude plugin install`. The reviewer gets `/looksie:review owner/repo#123`.
//! - Codex: Codex cannot install a plugin from the command line yet, so Looksie runs `codex mcp add` and puts the
//!   skill in `~/.agents/skills/looksie`, where Codex looks. The reviewer gets `$looksie owner/repo#123`.
//!
//! Nothing here runs unless the reviewer presses Connect (or reconnects after an update). What was connected is
//! remembered in agents.json, so an update to Looksie can refresh the plugin and skill to match.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths;

/// The plugin, marketplace and Claude skill names. The Claude command is `/looksie:review`.
const NAME: &str = "looksie";
/// Looksie's own version, from tauri.conf.json (the Rust crate's version never changes).
fn version() -> String {
    serde_json::from_str::<serde_json::Value>(include_str!("../tauri.conf.json")).ok().and_then(|v| v["version"].as_str().map(String::from)).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
    fn program(self) -> &'static str {
        self.id()
    }
    pub fn from_id(id: &str) -> Option<Agent> {
        match id {
            "claude" => Some(Agent::Claude),
            "codex" => Some(Agent::Codex),
            _ => None,
        }
    }
    /// How the reviewer starts a review from that agent.
    fn command(self) -> &'static str {
        match self {
            Agent::Claude => "/looksie:review",
            Agent::Codex => "$looksie",
        }
    }
}

// ---- The skill. One text for both agents; only the name, the agent's own name and Claude's tool names differ.

const SKILL_DESCRIPTION: &str = "Review GitHub pull requests with Looksie, the reviewer's own pull request helper: see what is waiting for review, walk through the decisions in a pull request, and write Looksie's AI layer (a plain-English summary, a question for each decision and checks of the author's claims) into the Looksie app. Use when the user asks to review a pull request (owner/repo#123 or a pull request link), asks what is waiting for their review, or mentions Looksie.";

const SKILL_BODY: &str = r#"# Looksie

Looksie turns a big pull request into the few decisions a reviewer has to make. Its tools come from the `looksie` MCP server (the Looksie app, run with `--mcp`). Looksie never posts anything to a pull request, and neither do you here.

## Write Looksie's AI layer (the usual job)

When the user names one or more pull requests (`owner/repo#123`, or a link), or asks you to review them:

1. Call `ai_prompt` with all of them in `pull_requests`. Several pull requests are reviewed together as one group.
2. Follow its instructions exactly and write the JSON answer it asks for. Every piece of evidence must be a line copied exactly from the changes it gives you; Looksie checks each one and throws away anything it cannot find. If you need more of a step's code, call `step_changes` with that step id.
3. Call `save_ai_layer` with the same `pull_requests`, your `answer`, and `from` set to "{AGENT}".
4. Tell the user in three short lines what you found, and that the answer is now in the Looksie app (it appears on that review by itself).

Do not approve, reject, comment on or change the pull request.

## Other questions

- "What needs my review?": call `review_queue` and list what is waiting, newest first, then ask which to look at.
- To explain a pull request without writing the AI layer: call `review`, then `step_changes` for the steps that matter, and answer from those.

## If something goes wrong

- No looksie tools: Looksie is not connected. Ask the user to open Looksie, then Settings, Your AI, and press Connect next to {AGENT}.
- An error about a private repository: private repositories need Looksie Pro (or its free trial). Pass the message on as it is.
- An error about signing in: the user needs to add their GitHub account in Looksie first.
"#;

fn skill_md(agent: Agent) -> String {
    let (name, extra) = match agent {
        // The plugin's tools are named mcp__plugin_<plugin>_<server>__<tool>; listing them lets the skill use them without asking each time.
        Agent::Claude => (
            "review",
            "argument-hint: \"owner/repo#123 [more pull requests]\"\nallowed-tools:\n  - mcp__plugin_looksie_looksie__review_queue\n  - mcp__plugin_looksie_looksie__review\n  - mcp__plugin_looksie_looksie__step_changes\n  - mcp__plugin_looksie_looksie__ai_prompt\n  - mcp__plugin_looksie_looksie__save_ai_layer\n",
        ),
        Agent::Codex => (NAME, ""),
    };
    let mut body = SKILL_BODY.replace("{AGENT}", agent.name());
    if agent == Agent::Claude {
        body += "\nPull requests named when this was started (may be empty): $ARGUMENTS\n";
    }
    // A JSON string is also a valid YAML string, so the description needs no hand quoting.
    let description = serde_json::to_string(SKILL_DESCRIPTION).unwrap_or_default();
    format!("---\nname: {name}\ndescription: {description}\n{extra}---\n\n{body}")
}

// ---- Files

/// The local marketplace Claude Code installs the plugin from: inside Looksie's own settings folder.
fn marketplace_dir() -> Result<PathBuf, String> {
    Ok(paths::config_dir()?.join("claude-marketplace"))
}

/// Where Codex (and other tools that read the open skills format) look for the user's own skills.
fn codex_skill_dir() -> Result<PathBuf, String> {
    // LOOKSIE_SKILLS_DIR is only for testing, so a test copy never touches the real ~/.agents.
    if let Some(d) = std::env::var_os("LOOKSIE_SKILLS_DIR") {
        return Ok(PathBuf::from(d).join(NAME));
    }
    Ok(dirs::home_dir().ok_or("No home folder on this computer.")?.join(".agents").join("skills").join(NAME))
}

fn this_program() -> Result<String, String> {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).map_err(|e| format!("Could not find Looksie itself: {e}"))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Could not write {}: {e}", d.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// Writes (or rewrites) the plugin and its marketplace. Returns the marketplace folder.
fn write_claude_plugin(program: &str) -> Result<PathBuf, String> {
    let root = marketplace_dir()?;
    let marketplace = serde_json::json!({
        "name": NAME,
        "owner": { "name": "Looksie", "url": "https://looksie.dev" },
        "description": "Looksie, on this computer.",
        "plugins": [{ "name": NAME, "source": "./plugin", "description": "Review pull requests with Looksie: /looksie:review owner/repo#123" }]
    });
    let plugin = serde_json::json!({
        "name": NAME,
        "displayName": "Looksie",
        "version": version(),
        "description": "Review pull requests with Looksie: see what is waiting, walk through the decisions, and write Looksie's AI layer into the app.",
        "author": { "name": "Looksie", "url": "https://looksie.dev" },
        "homepage": "https://looksie.dev"
    });
    let mcp = serde_json::json!({ "mcpServers": { NAME: { "command": program, "args": ["--mcp"] } } });
    let pretty = |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default() + "\n";
    write(&root.join(".claude-plugin").join("marketplace.json"), &pretty(&marketplace))?;
    let p = root.join("plugin");
    write(&p.join(".claude-plugin").join("plugin.json"), &pretty(&plugin))?;
    write(&p.join(".mcp.json"), &pretty(&mcp))?;
    write(&p.join("skills").join("review").join("SKILL.md"), &skill_md(Agent::Claude))?;
    Ok(root)
}

fn write_codex_skill() -> Result<PathBuf, String> {
    let d = codex_skill_dir()?;
    write(&d.join("SKILL.md"), &skill_md(Agent::Codex))?;
    // Codex's own extras: the name it shows, and that it may pick the skill by itself.
    write(&d.join("agents").join("openai.yaml"), "interface:\n  display_name: \"Looksie\"\n  short_description: \"Review pull requests with Looksie\"\npolicy:\n  allow_implicit_invocation: true\n")?;
    Ok(d)
}

// ---- Finding and running the agents' own command-line tools

/// Folders an agent's command-line tool is often in, beyond PATH. An app opened from the Mac's Finder or Dock gets a
/// very short PATH, so these matter there.
fn extra_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = dirs::home_dir() {
        for d in [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin", "bin"] {
            v.push(h.join(d));
        }
        if cfg!(windows) {
            if let Some(a) = std::env::var_os("APPDATA") {
                v.push(PathBuf::from(a).join("npm"));
            }
        }
    }
    if !cfg!(windows) {
        for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
            v.push(PathBuf::from(d));
        }
    }
    v
}

fn search_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    v.extend(extra_dirs());
    v
}

/// The full path to a command-line tool, or None if it is not on this computer.
fn find(program: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) { ["exe", "cmd", "bat"].iter().map(|e| format!("{program}.{e}")).collect() } else { vec![program.to_string()] };
    for d in search_dirs() {
        for n in &names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // Last try on Mac and Linux: the user's own login shell knows their PATH.
    if !cfg!(windows) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let out = Command::new(shell).args(["-lc", &format!("command -v {program}")]).output().ok()?;
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if out.status.success() && p.is_absolute() && p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Runs an agent's tool and gives back its output, or a plain error with what it said.
fn run(tool: &Path, args: &[&str]) -> Result<String, String> {
    // A .cmd (how npm installs Codex on Windows) runs directly too: Rust quotes its arguments safely for cmd.exe,
    // including paths with spaces, which `cmd /C` by hand gets wrong.
    let mut cmd = Command::new(tool);
    cmd.args(args).stdin(std::process::Stdio::null());
    // The tool's own folder and the usual ones go on PATH, so a tool written in Node finds node.
    let mut path = vec![tool.parent().map(Path::to_path_buf).unwrap_or_default()];
    path.extend(search_dirs());
    if let Ok(p) = std::env::join_paths(path) {
        cmd.env("PATH", p);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window flashes up.
    }
    let out = cmd.output().map_err(|e| format!("Could not run {}: {e}", tool.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        let said: String = text.trim().lines().rfind(|l| !l.trim().is_empty()).unwrap_or("no message").chars().take(300).collect();
        Err(said)
    }
}

// ---- What is connected

#[derive(Serialize, Deserialize, Default, Clone)]
struct Saved {
    #[serde(default)]
    claude: Option<String>,
    #[serde(default)]
    codex: Option<String>,
}

fn saved_path() -> Result<PathBuf, String> {
    Ok(paths::config_dir()?.join("agents.json"))
}

fn load() -> Saved {
    // A byte-order mark (Windows editors add one) is skipped rather than losing what was connected.
    let bytes = saved_path().ok().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).unwrap_or_default()
}

fn save(s: &Saved) -> Result<(), String> {
    write(&saved_path()?, &serde_json::to_string_pretty(s).map_err(|e| e.to_string())?)
}

/// What was connected, as "version|program": an update to Looksie, or Looksie moving, means it needs refreshing.
fn stamp(program: &str) -> String {
    format!("{}|{program}", version())
}

fn get(s: &Saved, a: Agent) -> &Option<String> {
    match a {
        Agent::Claude => &s.claude,
        Agent::Codex => &s.codex,
    }
}

fn set(s: &mut Saved, a: Agent, v: Option<String>) {
    match a {
        Agent::Claude => s.claude = v,
        Agent::Codex => s.codex = v,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    id: &'static str,
    name: &'static str,
    /// Its command-line tool is on this computer.
    found: bool,
    connected: bool,
    /// Connected, but to another copy of Looksie (moved, or a second install): Connect again fixes it.
    elsewhere: bool,
    /// What to type in that agent to start a review.
    command: &'static str,
    /// The same thing by hand, in a terminal.
    manual: Vec<String>,
}

pub fn states() -> Vec<AgentState> {
    let s = load();
    let program = this_program().unwrap_or_else(|_| "looksie".into());
    [Agent::Claude, Agent::Codex]
        .into_iter()
        .map(|a| AgentState {
            id: a.id(),
            name: a.name(),
            found: find(a.program()).is_some(),
            connected: get(&s, a).is_some(),
            elsewhere: get(&s, a).as_deref().is_some_and(|st| st.split_once('|').is_some_and(|(_, p)| p != program)),
            command: a.command(),
            manual: manual(a, &program),
        })
        .collect()
}

fn manual(a: Agent, program: &str) -> Vec<String> {
    match a {
        Agent::Claude => {
            let dir = marketplace_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            vec![format!("claude plugin marketplace add \"{dir}\""), format!("claude plugin install {NAME}@{NAME}")]
        }
        Agent::Codex => vec![format!("codex mcp add {NAME} -- \"{program}\" --mcp")],
    }
}

// ---- Connect and disconnect

pub fn connect(a: Agent) -> Result<(), String> {
    let program = this_program()?;
    let tool = find(a.program()).ok_or_else(|| format!("{} is not on this computer, or Looksie cannot find it. Install it, or connect by hand with the commands below.", a.name()))?;
    match a {
        Agent::Claude => {
            let dir = write_claude_plugin(&program)?;
            let dir_s = dir.to_string_lossy().into_owned();
            // An older Looksie told people to add the MCP server by hand; the plugin brings its own, so that one goes.
            let _ = run(&tool, &["mcp", "remove", NAME, "--scope", "user"]);
            // Added already (connected before, or by hand): refresh it instead.
            if run(&tool, &["plugin", "marketplace", "add", &dir_s]).is_err() {
                run(&tool, &["plugin", "marketplace", "update", NAME]).map_err(|e| format!("Claude Code did not add Looksie's plugin: {e}"))?;
            }
            run(&tool, &["plugin", "install", &format!("{NAME}@{NAME}"), "--scope", "user"]).map_err(|e| format!("Claude Code did not install Looksie's plugin: {e}"))?;
            // Already installed: install leaves the old copy, so make sure it is enabled and up to date.
            let _ = run(&tool, &["plugin", "update", &format!("{NAME}@{NAME}")]);
            let _ = run(&tool, &["plugin", "enable", &format!("{NAME}@{NAME}")]);
        }
        Agent::Codex => {
            write_codex_skill()?;
            // Replace any earlier entry, which may point at an old copy of Looksie.
            let _ = run(&tool, &["mcp", "remove", NAME]);
            run(&tool, &["mcp", "add", NAME, "--", &program, "--mcp"]).map_err(|e| format!("Codex did not add Looksie: {e}"))?;
        }
    }
    let mut s = load();
    set(&mut s, a, Some(stamp(&program)));
    save(&s)
}

pub fn disconnect(a: Agent) -> Result<(), String> {
    if let Some(tool) = find(a.program()) {
        match a {
            Agent::Claude => {
                let _ = run(&tool, &["plugin", "uninstall", &format!("{NAME}@{NAME}"), "--scope", "user"]);
                let _ = run(&tool, &["plugin", "marketplace", "remove", NAME]);
            }
            Agent::Codex => {
                let _ = run(&tool, &["mcp", "remove", NAME]);
            }
        }
    }
    match a {
        Agent::Claude => {
            if let Ok(d) = marketplace_dir() {
                let _ = std::fs::remove_dir_all(d);
            }
        }
        Agent::Codex => {
            if let Ok(d) = codex_skill_dir() {
                let _ = std::fs::remove_dir_all(d);
            }
        }
    }
    let mut s = load();
    set(&mut s, a, None);
    save(&s)
}

/// After an update, connected agents get the new skill. Runs at start-up, in the background; a failure leaves the
/// old connection as it was. Only for the same copy of Looksie updated in place: another copy (a second install,
/// a test build) never takes the connection over. Settings shows Connect again for that.
pub fn refresh() {
    let Ok(program) = this_program() else { return };
    let s = load();
    for a in [Agent::Claude, Agent::Codex] {
        if let Some(st) = get(&s, a) {
            if needs_refresh(st, &program) {
                let _ = connect(a);
            }
        }
    }
}

fn needs_refresh(saved: &str, program: &str) -> bool {
    saved != stamp(program) && saved.split_once('|').is_some_and(|(_, p)| p == program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_name_the_right_agent_and_tools() {
        let c = skill_md(Agent::Claude);
        assert!(c.starts_with("---\nname: review\ndescription: \"Review GitHub pull requests"));
        assert!(c.contains("$ARGUMENTS"));
        assert!(c.contains("mcp__plugin_looksie_looksie__save_ai_layer"));
        assert!(c.contains("`from` set to \"Claude Code\""));
        assert!(!c.contains("{AGENT}"));
        let x = skill_md(Agent::Codex);
        assert!(x.starts_with("---\nname: looksie\n"));
        assert!(x.contains("next to Codex"));
        assert!(!x.contains("allowed-tools"));
        assert!(SKILL_DESCRIPTION.len() < 1024);
    }

    /// A Windows user name with a space puts one in Looksie's path; a .cmd tool must still get it as one argument.
    #[cfg(windows)]
    #[test]
    fn cmd_tools_get_paths_with_spaces_whole() {
        let d = std::env::temp_dir().join("looksie agent test");
        std::fs::create_dir_all(&d).unwrap();
        let tool = d.join("fake tool.cmd");
        std::fs::write(&tool, "@echo off\r\necho [%1] [%2] [%3]\r\n").unwrap();
        let out = run(&tool, &["add", r"C:\Users\John Smith\AppData\Local\Looksie\looksie-app.exe", "--mcp"]).unwrap();
        assert_eq!(out.trim(), r#"[add] ["C:\Users\John Smith\AppData\Local\Looksie\looksie-app.exe"] [--mcp]"#);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn stamps_change_with_the_program() {
        assert_ne!(stamp("/a/looksie"), stamp("/b/looksie"));
        // The app's version (tauri.conf.json), not the crate's 0.1.0.
        assert!(version().split('.').count() == 3 && version() != "0.1.0");
        assert!(stamp("x").starts_with(&version()));
        // Updated in place: refresh. Same version: leave it. Another copy of Looksie: never take it over.
        assert!(needs_refresh("0.0.1|/Applications/Looksie.app/x", "/Applications/Looksie.app/x"));
        assert!(!needs_refresh(&stamp("/a"), "/a"));
        assert!(!needs_refresh("0.0.1|/a", "/test/b"));
    }
}
