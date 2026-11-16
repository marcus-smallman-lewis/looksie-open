//! Which GitHub accounts this computer can use, and which one is in use.
//!
//! Looksie finds accounts by itself, in three places:
//! - the GitHub CLI (`gh`): every account it is signed in to, on every server;
//! - tokens the user added in Looksie, kept in the OS keychain (one entry per account);
//! - the LOOKSIE_GITHUB_TOKEN environment variable.
//! Nothing is copied: a CLI account's token is asked for again each time it is needed.

use looksie_connectors::github::{clean_host, GitHub};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const KEYRING_SERVICE: &str = "dev.looksie.app";
/// Where the one token of earlier versions was kept. Moved to a per-account entry the first time it is seen.
const OLD_KEYRING_USER: &str = "github-token";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// "cli:github.com/octocat", "saved:github.acme.com/jo", "env:github.com/jo".
    pub id: String,
    pub host: String,
    pub login: String,
    /// "cli", "saved" or "env".
    pub source: String,
    /// The same, for showing: "found in the GitHub CLI".
    pub source_text: String,
    pub working: bool,
    /// What is wrong, when it is not working.
    pub problem: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SavedToken {
    host: String,
    login: String,
}

/// What Looksie remembers between runs. No tokens: those are in the keychain.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Remembered {
    active: Option<String>,
    #[serde(default)]
    tokens: Vec<SavedToken>,
}

fn file() -> Result<std::path::PathBuf, String> {
    Ok(crate::paths::config_dir()?.join("accounts.json"))
}

fn read() -> Remembered {
    file().ok().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(r: &Remembered) -> Result<(), String> {
    std::fs::write(file()?, serde_json::to_string_pretty(r).map_err(|e| e.to_string())?).map_err(|e| format!("Could not save: {e}"))
}

/// Places the GitHub CLI might be. Apps on macOS do not get the terminal's PATH, and Windows installs a .exe.
fn candidates(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if cfg!(windows) {
        out.push(format!("{name}.exe"));
        out.push(format!("{name}.cmd"));
        for dir in [std::env::var("ProgramFiles").ok().map(|p| format!("{p}\\GitHub CLI")), std::env::var("LOCALAPPDATA").ok().map(|p| format!("{p}\\Programs\\GitHub CLI"))].into_iter().flatten() {
            out.push(format!("{dir}\\{name}.exe"));
        }
    } else {
        for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/opt/local/bin"] {
            out.push(format!("{dir}/{name}"));
        }
        if let Ok(home) = std::env::var("HOME") {
            for dir in [".local/bin", "bin", ".nix-profile/bin"] {
                out.push(format!("{home}/{dir}/{name}"));
            }
        }
        // Last resort: ask the person's own shell where it is, the way a terminal would find it.
        if let Some(found) = ask_the_shell(name) {
            out.push(found);
        }
    }
    out
}

/// "command -v name" in a login shell. Only on macOS and Linux, and only for a plain tool name.
#[cfg(not(windows))]
fn ask_the_shell(name: &str) -> Option<String> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let shell = std::env::var("SHELL").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let out = std::process::Command::new(shell).args(["-lc", &format!("command -v {name}")]).stdin(std::process::Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().rev().map(str::trim).find(|l| l.starts_with('/')).map(str::to_string)
}

#[cfg(windows)]
fn ask_the_shell(_name: &str) -> Option<String> {
    None
}

fn keyring_user(host: &str, login: &str) -> String {
    format!("github-token:{host}/{login}")
}

fn keyring_get(user: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, user).ok()?.get_password().ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Runs the GitHub CLI and returns what it printed, whether or not it reported success.
fn gh(args: &[&str]) -> Option<(bool, String)> {
    for c in candidates("gh") {
        if let Ok(out) = crate::quiet(&c).args(args).output() {
            return Some((out.status.success(), String::from_utf8_lossy(&out.stdout).trim().to_string()));
        }
    }
    None
}

/// The accounts in `gh auth status --json hosts`: (host, login, working, active on its host).
pub fn parse_cli_accounts(json: &str) -> Vec<(String, String, bool, bool)> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut out = Vec::new();
    for (host, accounts) in v.get("hosts").and_then(Value::as_object).into_iter().flatten() {
        for a in accounts.as_array().into_iter().flatten() {
            let login = a.get("login").and_then(Value::as_str).unwrap_or("");
            if login.is_empty() {
                continue;
            }
            let working = a.get("state").and_then(Value::as_str) == Some("success");
            let active = a.get("active").and_then(Value::as_bool).unwrap_or(false);
            out.push((host.clone(), login.to_string(), working, active));
        }
    }
    out
}

fn account(source: &str, host: &str, login: &str, working: bool, problem: Option<String>) -> Account {
    let source_text = match source {
        "cli" => "found in the GitHub CLI",
        "saved" => "a token you added, kept in this computer's keychain",
        _ => "from the LOOKSIE_GITHUB_TOKEN environment variable",
    };
    Account { id: format!("{source}:{host}/{login}"), host: host.into(), login: login.into(), source: source.into(), source_text: source_text.into(), working, problem, active: false }
}

async fn who(token: &str, host: &str) -> Result<String, String> {
    GitHub::for_host(token.to_string(), host)?.viewer().await
}

/// Everything Looksie can find, with the one in use marked. Remembers the choice it lands on.
pub async fn discover(_app: &tauri::AppHandle) -> Vec<Account> {
    let mut mem = read();
    let mut out: Vec<Account> = Vec::new();

    // 1. The GitHub CLI. Newer versions list every account; older ones only hand over a token.
    let mut cli_default: Option<String> = None;
    match gh(&["auth", "status", "--json", "hosts"]) {
        Some((_, json)) if json.starts_with('{') => {
            for (host, login, working, active) in parse_cli_accounts(&json) {
                let problem = (!working).then(|| "The GitHub CLI says this sign-in no longer works. Run gh auth login again.".to_string());
                let a = account("cli", &host, &login, working, problem);
                if active && working && (cli_default.is_none() || host == "github.com") {
                    cli_default = Some(a.id.clone());
                }
                out.push(a);
            }
        }
        _ => {
            if let Some((true, token)) = gh(&["auth", "token"]) {
                if let Ok(login) = who(&token, "github.com").await {
                    let a = account("cli", "github.com", &login, true, None);
                    cli_default = Some(a.id.clone());
                    out.push(a);
                }
            }
        }
    }

    // 2. Tokens added in Looksie. An earlier version kept a single one: give it a proper entry.
    if let Some(old) = keyring_get(OLD_KEYRING_USER) {
        if let Ok(login) = who(&old, "github.com").await {
            let t = SavedToken { host: "github.com".into(), login };
            if keyring::Entry::new(KEYRING_SERVICE, &keyring_user(&t.host, &t.login)).and_then(|e| e.set_password(&old)).is_ok() {
                if !mem.tokens.contains(&t) {
                    mem.tokens.push(t);
                }
                if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, OLD_KEYRING_USER) {
                    let _ = e.delete_credential();
                }
            }
        }
    }
    let checks = mem.tokens.iter().map(|t| async move {
        match keyring_get(&keyring_user(&t.host, &t.login)) {
            None => account("saved", &t.host, &t.login, false, Some("Its token is no longer in the keychain. Add it again.".into())),
            Some(token) => match who(&token, &t.host).await {
                Ok(_) => account("saved", &t.host, &t.login, true, None),
                Err(_) => account("saved", &t.host, &t.login, false, Some("GitHub no longer accepts it. Replace the token.".into())),
            },
        }
    });
    out.extend(futures_util::future::join_all(checks).await);

    // 3. The environment variable, for people who run Looksie from a script.
    if let Some(token) = std::env::var("LOOKSIE_GITHUB_TOKEN").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        let host = std::env::var("LOOKSIE_GITHUB_HOST").ok().and_then(|h| clean_host(&h)).unwrap_or_else(|| "github.com".into());
        if let Ok(login) = who(&token, &host).await {
            out.push(account("env", &host, &login, true, None));
        }
    }

    // Which one is in use: the one chosen before if it still works, else the CLI's own choice, else the first that works.
    let usable = |id: &String| out.iter().any(|a| &a.id == id && a.working);
    let chosen = mem.active.clone().filter(usable).or(cli_default.filter(usable)).or_else(|| out.iter().find(|a| a.working).map(|a| a.id.clone()));
    for a in &mut out {
        a.active = Some(&a.id) == chosen.as_ref();
    }
    if mem.active != chosen {
        mem.active = chosen;
    }
    let _ = write(&mem);
    out
}

/// "cli:github.com/octocat" as ("cli", "github.com", "octocat").
fn split(id: &str) -> Option<(&str, &str, &str)> {
    let (source, rest) = id.split_once(':')?;
    let (host, login) = rest.rsplit_once('/')?;
    Some((source, host, login))
}

/// The token and server for the account in use. Asked for fresh each time; never cached.
pub fn active(_app: &tauri::AppHandle) -> Result<(String, String), String> {
    active_now()
}

/// The same, for `looksie --mcp`, which has no window.
pub fn active_now() -> Result<(String, String), String> {
    let none = || "Not signed in to GitHub yet. Open Looksie once and connect GitHub.".to_string();
    if let Some(id) = read().active {
        let (source, host, login) = split(&id).ok_or_else(none)?;
        let token = match source {
            "cli" => gh(&["auth", "token", "--hostname", host, "--user", login]).filter(|x| x.0).or_else(|| gh(&["auth", "token", "--hostname", host]).filter(|x| x.0)).map(|x| x.1),
            "saved" => keyring_get(&keyring_user(host, login)),
            _ => std::env::var("LOOKSIE_GITHUB_TOKEN").ok().map(|t| t.trim().to_string()),
        };
        return token.filter(|t| !t.is_empty()).map(|t| (t, host.to_string())).ok_or_else(|| format!("The sign-in for {login} on {host} is no longer available. Pick another account in Settings."));
    }
    // Nothing chosen yet (the accounts have not been listed): whatever the GitHub CLI would use.
    gh(&["auth", "token"]).filter(|x| x.0 && !x.1.is_empty()).map(|x| (x.1, "github.com".to_string())).ok_or_else(none)
}

pub fn choose(_app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let mut mem = read();
    mem.active = Some(id.to_string());
    write(&mem)
}

/// Checks a pasted token with its server, keeps it in the keychain, and makes it the account in use.
pub async fn add(_app: &tauri::AppHandle, token: &str, host: &str) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Paste a token first.".into());
    }
    let host = clean_host(host).ok_or_else(|| "That does not look like a server address. Use something like github.com or github.yourcompany.com.".to_string())?;
    let login = who(token, &host).await.map_err(|e| if e.contains("Could not reach") { format!("Could not reach {host}. Check the server address.") } else { format!("{host} did not accept that token.") })?;
    keyring::Entry::new(KEYRING_SERVICE, &keyring_user(&host, &login)).and_then(|e| e.set_password(token)).map_err(|e| format!("Could not save to the keychain: {e}"))?;
    let mut mem = read();
    let t = SavedToken { host: host.clone(), login: login.clone() };
    if !mem.tokens.contains(&t) {
        mem.tokens.push(t);
    }
    mem.active = Some(format!("saved:{host}/{login}"));
    write(&mem)
}

/// Forgets a token added in Looksie. Accounts from the GitHub CLI are not Looksie's to remove.
pub fn remove(_app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let (source, host, login) = split(id).ok_or("Unknown account.")?;
    if source != "saved" {
        return Err("That account comes from outside Looksie, so it cannot be removed here.".into());
    }
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &keyring_user(host, login)) {
        let _ = e.delete_credential();
    }
    let mut mem = read();
    mem.tokens.retain(|t| !(t.host == host && t.login == login));
    if mem.active.as_deref() == Some(id) {
        mem.active = None;
    }
    write(&mem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_account_the_cli_knows() {
        let json = r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"jo"},{"state":"success","active":false,"host":"github.com","login":"jo-bot"}],"github.acme.com":[{"state":"error","active":true,"host":"github.acme.com","login":"jsmith"}]}}"#;
        let mut found = parse_cli_accounts(json);
        found.sort();
        assert_eq!(found, vec![
            ("github.acme.com".to_string(), "jsmith".to_string(), false, true),
            ("github.com".to_string(), "jo".to_string(), true, true),
            ("github.com".to_string(), "jo-bot".to_string(), true, false),
        ]);
        assert!(parse_cli_accounts("not json").is_empty());
    }

    #[test]
    fn an_account_id_splits_back_into_its_parts() {
        assert_eq!(split("cli:github.com/jo"), Some(("cli", "github.com", "jo")));
        assert_eq!(split("saved:github.acme.com:8443/j.smith"), Some(("saved", "github.acme.com:8443", "j.smith")));
        assert_eq!(split("nonsense"), None);
    }
}
