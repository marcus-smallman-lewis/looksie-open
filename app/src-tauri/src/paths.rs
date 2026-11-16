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

/// AI answers, written by the app and by the reviewer's own agent through `looksie --mcp`.
pub fn ai_dir() -> Result<PathBuf, String> {
    Ok(cache_dir()?.join("ai"))
}
