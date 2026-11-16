//! AI answers kept on disk, so the app and the MCP server (`looksie --mcp`) share them.
//!
//! An answer belongs to a set of pull requests at particular commits. When new commits arrive the key changes,
//! so an old answer is never shown against new code.

use looksie_protocol::{AiLayer, Review};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    pub key: String,
    pub layer: AiLayer,
    /// Seconds since 1970.
    pub saved_at: u64,
}

/// The pull requests and their head commits, in a fixed order: the same review gives the same key wherever it was built.
pub fn key(review: &Review) -> String {
    let mut parts: Vec<String> = review
        .prs
        .iter()
        .map(|p| {
            let r = &p.summary.pr;
            format!("{}/{}/{}#{}@{}", r.host, r.owner, r.repo, r.number, p.head_sha).to_lowercase()
        })
        .collect();
    parts.sort();
    parts.join("+")
}

fn file(dir: &Path, key: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(key.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.json"))
}

pub fn load(dir: &Path, review: &Review) -> Option<Stored> {
    load_key(dir, &key(review))
}

/// The same, by key. The app works the key out itself, so it does not send a whole review just to ask.
pub fn load_key(dir: &Path, key: &str) -> Option<Stored> {
    let s: Stored = serde_json::from_str(&std::fs::read_to_string(file(dir, key)).ok()?).ok()?;
    (s.key == key).then_some(s)
}

#[cfg(test)]
mod tests {
    #[test]
    fn saved_answers_are_found_again_and_only_for_the_same_commits() {
        let dir = std::env::temp_dir().join(format!("looksie-ai-store-test-{}", std::process::id()));
        let k = "github.com/acme/api#12@abc+github.com/acme/web#3@def";
        let s = super::Stored { key: k.into(), layer: Default::default(), saved_at: 1 };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(super::file(&dir, k), serde_json::to_string(&s).unwrap()).unwrap();
        assert!(super::load_key(&dir, k).is_some());
        assert!(super::load_key(&dir, "github.com/acme/api#12@new").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub fn save(dir: &Path, review: &Review, layer: &AiLayer) -> Result<Stored, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not make the AI folder: {e}"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = Stored { key: key(review), layer: layer.clone(), saved_at: now };
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    // Write then rename, so the app never reads half a file.
    let target = file(dir, &s.key);
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Could not save the AI answer: {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("Could not save the AI answer: {e}"))?;
    Ok(s)
}
