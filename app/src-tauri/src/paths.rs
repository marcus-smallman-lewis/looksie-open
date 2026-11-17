//! Looksie's folders, found the same way Tauri finds them, so the window and `looksie --mcp` (which has no window) agree.

use std::path::PathBuf;

/// The app's identifier in tauri.conf.json.
pub const ID: &str = "dev.looksie.app";

/// Settings: accounts.json, licence.json, ai.json.
pub fn config_dir() -> Result<PathBuf, String> {
    let d = dirs::config_dir().ok_or("No settings folder on this computer.")?.join(ID);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Things that can be rebuilt: reviews and AI answers.
pub fn cache_dir() -> Result<PathBuf, String> {
    let d = dirs::cache_dir().ok_or("No cache folder on this computer.")?.join(ID);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// The reviewer's work in progress on each review (answers, notes, comments), shared with their own agent.
/// Settings, not cache: a clean-up must never delete someone's half-done review.
pub fn drafts_dir() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("drafts"))
}

/// Which review is open in the app, so "review the one I have open" works from Claude Code or Codex.
pub fn current_file() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("current-review.json"))
}

/// AI answers, written by the app and by the reviewer's own agent through `looksie --mcp`.
pub fn ai_dir() -> Result<PathBuf, String> {
    Ok(cache_dir()?.join("ai"))
}
