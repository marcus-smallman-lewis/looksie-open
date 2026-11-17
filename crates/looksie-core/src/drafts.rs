//! A reviewer's work in progress on a review (answers, notes, line comments, verdicts, summary), kept on disk so the
//! app and the reviewer's own agent (through `looksie --mcp`) share it. Start in Claude Code, finish in the app, or the
//! other way round. Like AI answers, a draft belongs to the exact commits that were reviewed.

use crate::ai_store;
use looksie_protocol::{Anchor, DraftComment, PrRef, Review, ReviewSubmission, ReviewVerdict, Side, Tier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The same shape the app keeps (`Saved` in app/src/types.ts), plus the key and when it was last changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Draft {
    pub key: String,
    /// Step id to "fine", "change" or "question".
    pub answers: BTreeMap<String, String>,
    /// Step id to a note for the author. A note always becomes a review comment.
    pub notes: BTreeMap<String, String>,
    /// "owner/repo#12|path|old or new|line" to the comment's text.
    pub comments: BTreeMap<String, String>,
    /// "owner/repo#12" to "approve", "request-changes" or "comment", when the reviewer chose one.
    pub verdicts: BTreeMap<String, String>,
    pub summary: String,
    /// "owner/repo#12" to the review already posted for it, so it is never posted twice.
    pub posted: BTreeMap<String, Posted>,
    /// Milliseconds since 1970, so the app and the agent can tell which copy is newer. Only ever goes up.
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Posted {
    pub url: String,
    pub commit: String,
    pub at: u64,
}

/// A change to a draft: only what changed. A null value removes that entry. Both the app and the agent write this way,
/// so neither can wipe what the other just saved.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Patch {
    pub answers: BTreeMap<String, Option<String>>,
    pub notes: BTreeMap<String, Option<String>>,
    pub comments: BTreeMap<String, Option<String>>,
    pub verdicts: BTreeMap<String, Option<String>>,
    pub summary: Option<String>,
    pub posted: BTreeMap<String, Option<Posted>>,
}

fn merge<T: Clone>(into: &mut BTreeMap<String, T>, from: &BTreeMap<String, Option<T>>) {
    for (k, v) in from {
        match v {
            Some(v) => {
                into.insert(k.clone(), v.clone());
            }
            None => {
                into.remove(k);
            }
        }
    }
}

pub fn pr_key(p: &PrRef) -> String {
    format!("{}/{}#{}", p.owner, p.repo, p.number)
}

fn file(dir: &Path, key: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(key.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{hex}.json"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The draft for this key: empty if there is none yet, an error if there is one that cannot be read
/// (so nothing is ever saved over a file that could not be read).
pub fn read(dir: &Path, key: &str) -> Result<Draft, String> {
    let path = file(dir, key);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Draft { key: key.to_string(), ..Draft::default() }),
        Err(e) => return Err(format!("Could not read the saved review: {e}")),
    };
    let d: Draft = serde_json::from_str(&text).map_err(|e| format!("The saved review could not be read ({e}); it was left as it is."))?;
    Ok(if d.key == key { d } else { Draft { key: key.to_string(), ..Draft::default() } })
}

/// For showing only: an unreadable draft shows as empty.
pub fn load_key(dir: &Path, key: &str) -> Draft {
    read(dir, key).unwrap_or_else(|_| Draft { key: key.to_string(), ..Draft::default() })
}

pub fn load(dir: &Path, review: &Review) -> Draft {
    load_key(dir, &ai_store::key(review))
}

/// Held while one writer reads, changes and saves a draft, so the app and the agent (separate processes) never both
/// read the same copy and drop each other's change. A lock left by a crash is taken over after ten seconds.
struct Lock(PathBuf);

impl Lock {
    fn take(path: PathBuf) -> Result<Lock, String> {
        for _ in 0..200 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Lock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|age| age.as_secs() > 10).unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("The saved review is busy; try again.".into())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Applies a change on top of what is on disk now, and saves. Returns the whole draft as saved.
pub fn apply(dir: &Path, key: &str, p: &Patch) -> Result<Draft, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let _lock = Lock::take(file(dir, key).with_extension("lock"))?;
    let mut d = read(dir, key)?;
    merge(&mut d.answers, &p.answers);
    merge(&mut d.notes, &p.notes);
    merge(&mut d.comments, &p.comments);
    merge(&mut d.verdicts, &p.verdicts);
    merge(&mut d.posted, &p.posted);
    if let Some(s) = &p.summary {
        d.summary = s.clone();
    }
    save(dir, &mut d)?;
    Ok(d)
}

/// Writes the draft, stamping the time (never earlier than the last stamp, whatever the clock does). Written to a
/// temporary file of its own first, so a reader never sees half a file and two writers never share one.
fn save(dir: &Path, draft: &mut Draft) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    draft.updated_at = now_ms().max(draft.updated_at + 1);
    let path = file(dir, &draft.key);
    let tmp = path.with_extension(format!("{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, serde_json::to_string(draft).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// What would be posted, one review per pull request. The same rules as the app's Send review screen:
/// a note always becomes a comment; "Needs changing" and "Question" without a note get a short one; the verdict is
/// the reviewer's choice, or Request changes if any answer says so, Approve if every step is Fine, else Comment.
/// On your own pull request the host only allows a comment.
pub fn submissions(review: &Review, d: &Draft, viewer: Option<&str>) -> Vec<ReviewSubmission> {
    review
        .prs
        .iter()
        .map(|p| {
            let pr = &p.summary.pr;
            let key = pr_key(pr);
            let mut comments = Vec::new();
            for s in review.steps.iter().filter(|s| s.files.first().map(|f| &f.pr == pr).unwrap_or(false)) {
                let a = d.answers.get(&s.id).map(String::as_str);
                let note = d.notes.get(&s.id).map(|n| n.trim()).unwrap_or("");
                if note.is_empty() && a != Some("change") && a != Some("question") {
                    continue;
                }
                let body = if !note.is_empty() { note.to_string() } else if a == Some("change") { format!("This needs changing: {}", s.title) } else { format!("Question about: {}", s.title) };
                let f = review.files.iter().find(|x| &x.pr == pr && x.path == s.files[0].path);
                let line = f.and_then(|f| f.hunks.iter().flat_map(|h| h.lines.iter()).find(|l| l.kind == looksie_protocol::LineKind::Add && l.new_no.is_some()).and_then(|l| l.new_no));
                let anchor = match (f, line) {
                    (Some(f), Some(n)) => Some(Anchor { path: f.path.clone(), side: Side::New, start_line: n, end_line: n }),
                    _ => None,
                };
                comments.push(DraftComment { anchor, body });
            }
            for (k, body) in &d.comments {
                let parts: Vec<&str> = k.splitn(4, '|').collect();
                if parts.len() == 4 && parts[0] == key && !body.trim().is_empty() {
                    if let Ok(n) = parts[3].parse::<u32>() {
                        let side = if parts[2] == "old" { Side::Old } else { Side::New };
                        comments.push(DraftComment { anchor: Some(Anchor { path: parts[1].to_string(), side, start_line: n, end_line: n }), body: body.clone() });
                    }
                }
            }
            let touching: Vec<_> = review.steps.iter().filter(|s| s.files.iter().any(|f| &f.pr == pr)).collect();
            let suggested = if touching.iter().any(|s| d.answers.get(&s.id).map(String::as_str) == Some("change")) {
                ReviewVerdict::RequestChanges
            } else if !touching.is_empty() && touching.iter().all(|s| d.answers.get(&s.id).map(String::as_str) == Some("fine")) {
                ReviewVerdict::Approve
            } else {
                ReviewVerdict::Comment
            };
            let chosen = match d.verdicts.get(&key).map(String::as_str) {
                Some("approve") => Some(ReviewVerdict::Approve),
                Some("request-changes") => Some(ReviewVerdict::RequestChanges),
                Some("comment") => Some(ReviewVerdict::Comment),
                _ => None,
            };
            let own = viewer.map(|v| v.eq_ignore_ascii_case(&p.summary.author)).unwrap_or(false);
            let verdict = if own { ReviewVerdict::Comment } else { chosen.unwrap_or(suggested) };
            ReviewSubmission { pr: pr.clone(), commit: p.head_sha.clone(), verdict, summary: d.summary.trim().to_string(), comments }
        })
        .collect()
}

/// A review with no verdict to give, no summary and no comments has nothing in it, and the host would refuse it.
pub fn sendable(s: &ReviewSubmission) -> bool {
    s.verdict == ReviewVerdict::Approve || !s.summary.is_empty() || !s.comments.is_empty()
}

/// How far through the decisions the reviewer is.
pub fn progress(review: &Review, d: &Draft) -> (usize, usize) {
    let decisions: Vec<_> = review.steps.iter().filter(|s| s.tier == Tier::Decide).collect();
    (decisions.iter().filter(|s| d.answers.contains_key(&s.id)).count(), decisions.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_from_two_writers_both_survive() {
        let dir = std::env::temp_dir().join(format!("looksie-drafts-test-{}", std::process::id()));
        let key = "github/a/b#1@abc";
        let mut app = Patch::default();
        app.answers.insert("step-1".into(), Some("fine".into()));
        let first = apply(&dir, key, &app).unwrap();
        let mut agent = Patch::default();
        agent.answers.insert("step-2".into(), Some("change".into()));
        agent.notes.insert("step-2".into(), Some("Add a test.".into()));
        let second = apply(&dir, key, &agent).unwrap();
        assert_eq!(second.answers.len(), 2, "the agent's answer did not wipe the app's");
        assert!(second.updated_at > first.updated_at);
        let mut clear = Patch::default();
        clear.answers.insert("step-1".into(), None);
        assert_eq!(apply(&dir, key, &clear).unwrap().answers.keys().collect::<Vec<_>>(), vec!["step-2"]);
        assert!(load_key(&dir, "github/a/b#1@def").answers.is_empty(), "new commits start a clean sheet");
        // A file that cannot be read is never written over.
        std::fs::write(file(&dir, key), "{ not json").unwrap();
        assert!(apply(&dir, key, &app).is_err());
        assert_eq!(std::fs::read_to_string(file(&dir, key)).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
