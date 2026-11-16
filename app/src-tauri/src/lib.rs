//! The desktop app's backend: a thin layer of commands the UI calls.
//! Network calls go through `looksie-connectors`; the thinking happens in `looksie-engine`.

mod accounts;
mod agents;
mod paths;
mod store;

use looksie_connectors::{github::GitHub, Connector};
use looksie_engine::licence::{self, LicenceState, LicenceStatus};
use looksie_core::{load_review, parse_pr};
use looksie_protocol::{PrSummary, Review, ReviewSubmission};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Runs a helper program without flashing a console window on Windows.
pub(crate) fn quiet(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// The connection to GitHub for the account in use.
fn github(app: &tauri::AppHandle) -> Result<GitHub, String> {
    let (token, host) = accounts::active(app)?;
    GitHub::for_host(token, &host)
}

/// Every GitHub account Looksie can find on this computer, with the one in use marked.
#[tauri::command]
async fn list_accounts(app: tauri::AppHandle) -> Vec<accounts::Account> {
    accounts::discover(&app).await
}

#[tauri::command]
async fn use_account(app: tauri::AppHandle, id: String) -> Result<Vec<accounts::Account>, String> {
    accounts::choose(&app, &id)?;
    Ok(accounts::discover(&app).await)
}

/// Checks a pasted token with its server and keeps it in the OS keychain (Windows Credential Manager, macOS Keychain).
#[tauri::command]
async fn add_account(app: tauri::AppHandle, token: String, host: String) -> Result<Vec<accounts::Account>, String> {
    accounts::add(&app, &token, &host).await?;
    Ok(accounts::discover(&app).await)
}

#[tauri::command]
async fn remove_account(app: tauri::AppHandle, id: String) -> Result<Vec<accounts::Account>, String> {
    accounts::remove(&app, &id)?;
    Ok(accounts::discover(&app).await)
}

#[tauri::command]
async fn list_inbox(app: tauri::AppHandle) -> Result<Vec<PrSummary>, String> {
    github(&app)?.list_my_prs().await
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn licence_file(_app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(paths::config_dir()?.join("licence.json"))
}

/// The trial's start is also kept in the OS keychain, so deleting the settings or reinstalling does not start a new trial.
const TRIAL_KEYCHAIN: (&str, &str) = ("dev.looksie.app", "trial-started");

fn read_licence(_app: &tauri::AppHandle) -> LicenceState {
    read_licence_now()
}

fn read_licence_now() -> LicenceState {
    let mut state: LicenceState = paths::config_dir().map(|d| d.join("licence.json")).ok().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let kept = keyring::Entry::new(TRIAL_KEYCHAIN.0, TRIAL_KEYCHAIN.1).ok().and_then(|e| e.get_password().ok()).and_then(|t| t.trim().parse::<u64>().ok());
    match (state.trial_started, kept) {
        (None, Some(t)) => state.trial_started = Some(t),
        (Some(t), None) => { let _ = keyring::Entry::new(TRIAL_KEYCHAIN.0, TRIAL_KEYCHAIN.1).and_then(|e| e.set_password(&t.to_string())); }
        (Some(a), Some(b)) if b < a => state.trial_started = Some(b),
        _ => {}
    }
    // A key counts only if it is the private test key or the store activated it on this computer.
    // (An early preview accepted any key; those are dropped here.)
    if state.key.as_deref().map(|k| !store::is_known_key(k)).unwrap_or(false) && state.activation.is_none() {
        state.key = None;
    }
    state
}

fn write_licence(app: &tauri::AppHandle, state: &LicenceState) -> Result<(), String> {
    std::fs::write(licence_file(app)?, serde_json::to_string_pretty(state).map_err(|e| e.to_string())?).map_err(|e| format!("Could not save: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Shop {
    status: LicenceStatus,
    price_monthly: &'static str,
    price_yearly: &'static str,
    offer: &'static str,
    checkout_monthly: &'static str,
    checkout_yearly: &'static str,
    portal: &'static str,
    trial_days: u64,
}

fn shop(app: &tauri::AppHandle) -> Shop {
    Shop { status: licence::status(&read_licence(app), now()), price_monthly: store::PRICE_MONTHLY, price_yearly: store::PRICE_YEARLY, offer: store::OFFER, checkout_monthly: store::POLAR.checkout_monthly, checkout_yearly: store::POLAR.checkout_yearly, portal: store::POLAR.portal, trial_days: licence::TRIAL_DAYS }
}

/// How many launch-offer places are left, as published on the website. None when it cannot be read.
#[derive(Serialize, serde::Deserialize)]
struct Offer {
    left: u32,
    max: u32,
}

#[tauri::command]
async fn offer_left() -> Option<Offer> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(6)).build().ok()?;
    let res = client.get(store::OFFER_URL).header("Cache-Control", "no-cache").send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Offer>().await.ok()
}

/// The plan this computer is on, and what Pro costs.
#[tauri::command]
fn licence_status(app: tauri::AppHandle) -> Shop {
    shop(&app)
}

/// Starts the free trial. No card, no account. It can only be started once.
#[tauri::command]
fn start_trial(app: tauri::AppHandle) -> Result<Shop, String> {
    let mut state = read_licence(&app);
    if state.trial_started.is_some() {
        return Err("The free trial has already been used on this computer.".into());
    }
    state.trial_started = Some(now());
    write_licence(&app, &state)?;
    let _ = keyring::Entry::new(TRIAL_KEYCHAIN.0, TRIAL_KEYCHAIN.1).and_then(|e| e.set_password(&now().to_string()));
    Ok(shop(&app))
}

/// Checks a pasted key with the store and uses one of its 3 devices for this computer.
#[tauri::command]
async fn activate_licence(app: tauri::AppHandle, key: String) -> Result<Shop, String> {
    let key = licence::clean_key(&key).ok_or_else(|| "That does not look like a Looksie licence key.".to_string())?;
    let mut state = read_licence(&app);
    if state.key.as_deref() == Some(key.as_str()) && licence::status(&state, now()).plan == "pro" {
        return Ok(shop(&app)); // already in use here
    }
    if store::is_known_key(&key) {
        state.activation = None;
        state.checked_at = None;
    } else {
        let activation = store::activate(&key).await?;
        // A different key was in use here: give its device back.
        if let (Some(old), Some(old_act)) = (state.key.clone(), state.activation.clone()) {
            store::deactivate(&old, &old_act).await;
        }
        state.activation = Some(activation);
        state.checked_at = Some(now());
    }
    state.key = Some(key);
    write_licence(&app, &state)?;
    Ok(shop(&app))
}

/// Takes the key off this computer and frees its device, so it can be used on another.
#[tauri::command]
async fn remove_licence(app: tauri::AppHandle) -> Result<Shop, String> {
    let mut state = read_licence(&app);
    if let (Some(key), Some(act)) = (state.key.clone(), state.activation.clone()) {
        store::deactivate(&key, &act).await;
    }
    state.key = None;
    state.activation = None;
    state.checked_at = None;
    write_licence(&app, &state)?;
    Ok(shop(&app))
}

/// Once a week, asks the store whether the key is still good. A cancelled or ended subscription stops counting here.
/// Offline, nothing changes: the key keeps working for the grace period.
#[tauri::command]
async fn recheck_licence(app: tauri::AppHandle) -> Shop {
    let mut state = read_licence(&app);
    if licence::needs_recheck(&state, now()) {
        if let (Some(key), Some(act)) = (state.key.clone(), state.activation.clone()) {
            match store::validate(&key, &act).await {
                Ok(store::Check::Good) => state.checked_at = Some(now()),
                Ok(store::Check::Gone) => {
                    state.key = None;
                    state.activation = None;
                    state.checked_at = None;
                }
                Err(_) => {}
            }
            let _ = write_licence(&app, &state);
        }
    }
    shop(&app)
}

/// Reviews built before, kept on disk so opening one again is instant. Thrown away when the commits or Looksie change.
mod cache {
    use looksie_protocol::Review;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use tauri::Manager;

    #[derive(Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Entry {
        /// The latest commit of each pull request, in the review's order.
        pub heads: Vec<String>,
        pub built_at: u64,
        pub version: String,
        pub file: String,
    }

    fn dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
        let d = app.path().app_cache_dir().ok()?.join("reviews");
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    }

    pub fn index(app: &tauri::AppHandle) -> BTreeMap<String, Entry> {
        let version = looksie_engine::ENGINE_VERSION.to_string();
        dir(app)
            .and_then(|d| std::fs::read_to_string(d.join("index.json")).ok())
            .and_then(|t| serde_json::from_str::<BTreeMap<String, Entry>>(&t).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, e)| e.version == version)
            .collect()
    }

    /// The key for a set of pull requests, whatever order they were picked in.
    pub fn key(prs: &[String]) -> String {
        let mut k: Vec<String> = prs.iter().map(|p| p.trim().to_lowercase()).collect();
        k.sort();
        k.join("+")
    }

    pub fn get(app: &tauri::AppHandle, key: &str, heads: &[String]) -> Option<Review> {
        let e = index(app).remove(key)?;
        let mut a = e.heads.clone();
        let mut b = heads.to_vec();
        a.sort();
        b.sort();
        if a != b || b.iter().any(String::is_empty) {
            return None;
        }
        serde_json::from_str(&std::fs::read_to_string(dir(app)?.join(&e.file)).ok()?).ok()
    }

    pub fn put(app: &tauri::AppHandle, key: &str, review: &Review) {
        let Some(d) = dir(app) else { return };
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        let file = format!("{:016x}.json", h.finish());
        let Ok(text) = serde_json::to_string(review) else { return };
        if std::fs::write(d.join(&file), text).is_err() {
            return;
        }
        let mut all = index(app);
        all.insert(key.to_string(), Entry { heads: review.prs.iter().map(|p| p.head_sha.clone()).collect(), built_at: super::now(), version: looksie_engine::ENGINE_VERSION.to_string(), file });
        // Keep the newest 200; drop the rest and their files.
        if all.len() > 200 {
            let mut by_age: Vec<(String, u64, String)> = all.iter().map(|(k, e)| (k.clone(), e.built_at, e.file.clone())).collect();
            by_age.sort_by_key(|x| x.1);
            for (k, _, f) in by_age.into_iter().take(all.len() - 200) {
                all.remove(&k);
                let _ = std::fs::remove_file(d.join(f));
            }
        }
        if let Ok(t) = serde_json::to_string(&all) {
            let _ = std::fs::write(d.join("index.json"), t);
        }
    }
}

/// Which reviews are ready to open instantly, and for which commits. The inbox marks those rows.
#[tauri::command]
fn cached_reviews(app: tauri::AppHandle) -> std::collections::BTreeMap<String, cache::Entry> {
    cache::index(&app)
}

/// Builds a review. With `heads` (the latest commits the inbox knows about) and not `fresh`, a review prepared
/// earlier for exactly those commits is returned straight away instead.
#[tauri::command]
async fn build_review(app: tauri::AppHandle, prs: Vec<String>, group_name: Option<String>, heads: Option<Vec<String>>, fresh: Option<bool>) -> Result<Review, String> {
    let refs = prs.iter().map(|p| parse_pr(p)).collect::<Result<Vec<_>, _>>()?;
    let key = cache::key(&prs);
    let kept = match (&heads, fresh.unwrap_or(false)) {
        (Some(h), false) => cache::get(&app, &key, h),
        _ => None,
    };
    let review = match kept {
        Some(r) => r,
        None => {
            let r = load_review(&github(&app)?, &refs, group_name).await?;
            cache::put(&app, &key, &r);
            r
        }
    };
    // Private repositories are the paid part. The UI turns this message into the upgrade screen.
    let private: Vec<String> = review.prs.iter().filter(|p| p.summary.private).map(|p| p.summary.pr.label()).collect();
    if !private.is_empty() && !licence::can_open_private(&licence::status(&read_licence(&app), now())) {
        return Err(format!("pro-required:{}", private.join(", ")));
    }
    Ok(review)
}

/// Which of these pull requests look like one piece of work. Explicit links only; nothing is fetched.
#[tauri::command]
fn suggest_groups(prs: Vec<PrSummary>) -> Vec<looksie_protocol::GroupSuggestion> {
    looksie_engine::groups::suggest(&prs)
}

/// Exactly what would be sent to GitHub for this review. Sends nothing.
#[tauri::command]
fn review_payload(review: ReviewSubmission) -> serde_json::Value {
    looksie_connectors::github::review_payload(&review)
}

/// Posts one review. The UI only calls this from the Submit button, after a confirmation.
#[tauri::command]
async fn submit_review(app: tauri::AppHandle, review: ReviewSubmission) -> Result<String, String> {
    github(&app)?.submit_review(&review).await
}

// ---- AI: the reviewer's own Anthropic or OpenAI API key in the app, or their own agent through `looksie --mcp`.

/// API keys live in the OS keychain, next to GitHub tokens, one entry per company. Never in a file.
fn ai_key_entry(p: looksie_ai::Provider) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dev.looksie.app", &format!("ai-key:{}", p.id()))
}

fn ai_key(p: looksie_ai::Provider) -> Option<String> {
    ai_key_entry(p).ok()?.get_password().ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

/// What is remembered about AI besides the keys: the model, which also says which company.
#[derive(Default, Serialize, Deserialize)]
struct AiChoice {
    #[serde(default)]
    model: Option<String>,
}

fn ai_file() -> Result<std::path::PathBuf, String> {
    Ok(paths::config_dir()?.join("ai.json"))
}

fn ai_choice() -> AiChoice {
    ai_file().ok().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn ai_model() -> &'static looksie_ai::Model {
    looksie_ai::model(ai_choice().model.as_deref().unwrap_or(""))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiModel {
    id: &'static str,
    name: &'static str,
    note: &'static str,
    provider: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiProvider {
    id: &'static str,
    name: &'static str,
    /// "Claude" or "GPT", for the Ask button.
    assistant: &'static str,
    has_key: bool,
    /// The last four characters, to show which key is in use.
    key_hint: Option<String>,
    key_start: &'static str,
    keys_url: &'static str,
    keys_page: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSetup {
    /// Whether the company of the chosen model has a key, so the app can ask.
    has_key: bool,
    key_hint: Option<String>,
    model: &'static str,
    /// The company of the chosen model.
    provider: &'static str,
    providers: Vec<AiProvider>,
    models: Vec<AiModel>,
}

fn hint(k: &str) -> String {
    k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect()
}

fn ai_setup_now() -> AiSetup {
    let m = ai_model();
    let providers: Vec<AiProvider> = looksie_ai::PROVIDERS
        .iter()
        .map(|&p| {
            let key = ai_key(p);
            AiProvider { id: p.id(), name: p.name(), assistant: p.assistant(), has_key: key.is_some(), key_hint: key.as_deref().map(hint), key_start: p.key_start(), keys_url: p.keys_url(), keys_page: p.keys_page() }
        })
        .collect();
    let chosen = providers.iter().find(|p| p.id == m.provider.id());
    AiSetup {
        has_key: chosen.is_some_and(|p| p.has_key),
        key_hint: chosen.and_then(|p| p.key_hint.clone()),
        model: m.id,
        provider: m.provider.id(),
        providers,
        models: looksie_ai::MODELS.iter().map(|m| AiModel { id: m.id, name: m.name, note: m.note, provider: m.provider.id() }).collect(),
    }
}

/// How AI is set up on this computer. Looksie never reads anyone's Claude.ai or ChatGPT sign-in.
#[tauri::command]
fn ai_setup() -> AiSetup {
    ai_setup_now()
}

fn save_model(id: &str) -> Result<(), String> {
    let choice = AiChoice { model: Some(id.to_string()) };
    std::fs::write(ai_file()?, serde_json::to_string_pretty(&choice).map_err(|e| e.to_string())?).map_err(|e| format!("Could not save: {e}"))
}

/// Checks a pasted API key with its company (without spending anything), then keeps it in the keychain.
/// `provider` is the company the reviewer is setting up; the key has to match it.
#[tauri::command]
async fn set_ai_key(provider: String, key: String) -> Result<AiSetup, String> {
    let p = looksie_ai::Provider::from_id(&provider).ok_or("Unknown AI company.")?;
    let key = key.trim().to_string();
    let looks = looksie_ai::Provider::of_key(&key);
    if key.len() < 20 || key.contains(char::is_whitespace) || looks.is_none() {
        return Err(format!("That does not look like an {} API key. They start with {}.", p.name(), p.key_start()));
    }
    if looks != Some(p) {
        let other = looks.unwrap_or(p);
        return Err(format!("That is an {} key. Pick {} above to use it.", other.name(), other.name()));
    }
    looksie_ai::check_key(p, &key).await?;
    ai_key_entry(p).and_then(|e| e.set_password(&key)).map_err(|e| format!("Could not save to the keychain: {e}"))?;
    // Saving a key for the other company switches to it: that is what the reviewer is setting up.
    if ai_model().provider != p {
        save_model(looksie_ai::default_for(p).id)?;
    }
    Ok(ai_setup_now())
}

#[tauri::command]
fn remove_ai_key(provider: String) -> AiSetup {
    if let Some(p) = looksie_ai::Provider::from_id(&provider) {
        if let Ok(e) = ai_key_entry(p) {
            let _ = e.delete_credential();
        }
    }
    ai_setup_now()
}

#[tauri::command]
fn set_ai_model(model: String) -> Result<AiSetup, String> {
    let m = looksie_ai::MODELS.iter().find(|m| m.id == model).ok_or("Unknown model.")?;
    save_model(m.id)?;
    Ok(ai_setup_now())
}

/// The exact prompt that would be sent to the AI for this review. Sends nothing.
#[tauri::command]
fn ai_prompt(review: Review) -> String {
    looksie_engine::ai::prompt(&review)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiEstimate {
    chars: usize,
    /// Rough US dollars with the chosen model.
    dollars: f64,
}

/// How big the prompt is, and roughly what it costs with the chosen model.
#[tauri::command]
fn ai_estimate(review: Review) -> AiEstimate {
    let chars = looksie_engine::ai::prompt(&review).len();
    AiEstimate { chars, dollars: looksie_ai::rough_cost(ai_model().id, chars) }
}

/// Asks the chosen model with the reviewer's own API key, keeps only answers that point at real lines, and saves them.
#[tauri::command]
async fn run_ai(review: Review) -> Result<looksie_protocol::AiLayer, String> {
    let model = ai_model();
    let key = ai_key(model.provider).ok_or_else(|| format!("Add your {} API key in Settings, Your AI, first.", model.provider.name()))?;
    let prompt = looksie_engine::ai::prompt(&review);
    let answer = looksie_ai::ask(&key, model.id, &prompt).await?;
    let layer = looksie_engine::ai::read(&review, &answer, model.name);
    looksie_core::ai_store::save(&paths::ai_dir()?, &review, &layer)?;
    Ok(layer)
}

/// The AI answer saved for a review at these commits, by the app or by the reviewer's own agent.
/// `key` is `ai_store::key` worked out by the app: host/owner/repo#number@headsha, lower case, sorted, joined by +.
#[tauri::command]
fn saved_ai(key: String) -> Option<looksie_core::ai_store::Stored> {
    looksie_core::ai_store::load_key(&paths::ai_dir().ok()?, &key)
}

/// The reviewer's own Claude Code and Codex: whether each is on this computer and connected to Looksie.
#[tauri::command]
async fn agent_states() -> Vec<agents::AgentState> {
    tauri::async_runtime::spawn_blocking(agents::states).await.unwrap_or_default()
}

/// Connects Claude Code (a Looksie plugin with the skill and the MCP server) or Codex (the MCP server and the skill).
#[tauri::command]
async fn connect_agent(id: String) -> Result<Vec<agents::AgentState>, String> {
    let a = agents::Agent::from_id(&id).ok_or("Unknown agent.")?;
    tauri::async_runtime::spawn_blocking(move || agents::connect(a).map(|_| agents::states())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn disconnect_agent(id: String) -> Result<Vec<agents::AgentState>, String> {
    let a = agents::Agent::from_id(&id).ok_or("Unknown agent.")?;
    tauri::async_runtime::spawn_blocking(move || agents::disconnect(a).map(|_| agents::states())).await.map_err(|e| e.to_string())?
}

/// `looksie --mcp`: the reviewer's own Claude Code or Codex uses Looksie as a tool, and can save its answer for the app.
/// Private repositories need Pro here too, exactly as in the window.
pub fn run_mcp() -> i32 {
    let result = tauri::async_runtime::block_on(async {
        let (token, host) = accounts::active_now()?;
        let gh = GitHub::for_host(token, &host)?;
        let state = read_licence_now();
        let private_ok = licence::can_open_private(&licence::status(&state, now()));
        looksie_core::mcp::serve(gh, Some(paths::ai_dir()?), private_ok).await
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Puts text on the clipboard, through the system rather than the web view (which only allows it from a real click).
#[tauri::command]
fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Opens a web page in the user's browser. Only https addresses.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Only https links can be opened.".into());
    }
    let result = if cfg!(windows) {
        quiet("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&url).spawn()
    } else {
        Command::new("xdg-open").arg(&url).spawn()
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Updates: checked against a signed latest.json on GitHub Releases. Nothing installs without the user's click.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // A Looksie update refreshes the skill and plugin of any agent connected before, in the background.
        .setup(|_| {
            std::thread::spawn(agents::refresh);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![cached_reviews, list_accounts, use_account, add_account, remove_account, list_inbox, suggest_groups, build_review, licence_status, recheck_licence, offer_left, start_trial, activate_licence, remove_licence, review_payload, submit_review, ai_setup, set_ai_key, remove_ai_key, set_ai_model, ai_prompt, ai_estimate, run_ai, saved_ai, agent_states, connect_agent, disconnect_agent, copy_text, open_url])
        .run(tauri::generate_context!())
        .expect("Looksie could not start");
}
