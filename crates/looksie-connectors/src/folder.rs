//! A folder of local git repositories, described by a `manifest.json`. Makes no network calls.
//!
//! This is how the demo runs offline: `demo/generate.py` builds the repositories and the manifest, and this
//! connector answers the same questions GitHub would. It reads with `git` and cannot post anything.

use crate::github::{linked_issue_numbers, links_of, looks_like_agent};
use crate::{At, Connector};
use looksie_protocol::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Folder {
    root: PathBuf,
    org: String,
    repos: Vec<Value>,
}

/// One pull request in the manifest, with the number GitHub would give it.
struct Entry<'a> {
    repo: &'a Value,
    pr: &'a Value,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Folder {
    /// `folder` holds `manifest.json` and `out/<repository>/`.
    pub fn open(folder: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(folder.join("manifest.json")).map_err(|e| format!("No manifest.json in {}: {e}", folder.display()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("manifest.json is not valid: {e}"))?;
        Ok(Folder { root: folder.join("out"), org: s(&v, "org"), repos: v.get("repos").and_then(Value::as_array).cloned().unwrap_or_default() })
    }

    /// Issues are opened before pull requests, so a repository with an issue starts its pull requests at 2.
    fn first_number(repo: &Value) -> u64 {
        if repo.get("issue").map(|i| !i.is_null()).unwrap_or(false) { 2 } else { 1 }
    }

    fn all(&self) -> Vec<(PrRef, Entry<'_>)> {
        let mut out = Vec::new();
        for repo in &self.repos {
            for (i, pr) in repo.get("prs").and_then(Value::as_array).into_iter().flatten().enumerate() {
                out.push((PrRef::github(&self.org, &s(repo, "name"), Self::first_number(repo) + i as u64), Entry { repo, pr }));
            }
        }
        out
    }

    fn find(&self, pr: &PrRef) -> Result<Entry<'_>, String> {
        self.all().into_iter().find(|(r, _)| r == pr).map(|(_, e)| e).ok_or_else(|| format!("{} is not in this folder", pr.label()))
    }

    fn git(&self, repo: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git").arg("-C").arg(self.root.join(repo)).args(["-c", "core.quotepath=off"]).args(args).output().map_err(|e| format!("Could not run git: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn summary(&self, pr: &PrRef, e: &Entry) -> PrSummary {
        let (title, body) = (s(e.pr, "title"), s(e.pr, "body"));
        let range = format!("{}...{}", s(e.pr, "base"), s(e.pr, "branch"));
        let (mut additions, mut deletions, mut changed_files) = (0, 0, 0);
        for line in self.git(&pr.repo, &["diff", "--numstat", &range]).unwrap_or_default().lines() {
            let mut cols = line.split('\t');
            additions += cols.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
            deletions += cols.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
            changed_files += 1;
        }
        let author = "harbour-bot".to_string();
        PrSummary {
            pr: pr.clone(),
            agent: looks_like_agent(&author, &body),
            author,
            private: false,
            additions,
            deletions,
            changed_files,
            updated_at: "2026-11-01T09:00:00Z".into(),
            url: format!("https://github.com/{}/{}/pull/{}", pr.owner, pr.repo, pr.number),
            relation: "review-requested".into(),
            links: links_of(pr, &title, &body),
            head_sha: self.git(&pr.repo, &["rev-parse", &s(e.pr, "branch")]).map(|t| t.trim().to_string()).unwrap_or_default(),
            title,
        }
    }
}

/// Splits `git diff` output into one patch per file, in the shape GitHub sends: from the first "@@" line on.
fn patches(diff: &str) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    let mut in_hunks = false;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(lines) = current.take() {
                out.push(if lines.is_empty() { None } else { Some(lines.join("\n")) });
            }
            current = Some(Vec::new());
            in_hunks = false;
            continue;
        }
        if line.starts_with("@@") {
            in_hunks = true;
        }
        if in_hunks {
            if let Some(lines) = current.as_mut() {
                lines.push(line);
            }
        }
    }
    if let Some(lines) = current {
        out.push(if lines.is_empty() { None } else { Some(lines.join("\n")) });
    }
    out
}

impl Connector for Folder {
    fn kind(&self) -> &'static str {
        "folder"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { inline_comments: false, multi_line: false, suggestions: false, verdicts: false, reply: false }
    }

    async fn list_my_prs(&self) -> Result<Vec<PrSummary>, String> {
        Ok(self.all().iter().map(|(pr, e)| self.summary(pr, e)).collect())
    }

    async fn get_pr(&self, pr: &PrRef) -> Result<PrDetail, String> {
        let e = self.find(pr)?;
        let (base, branch) = (s(e.pr, "base"), s(e.pr, "branch"));
        let summary = self.summary(pr, &e);
        let body = s(e.pr, "body");
        let sha = |what: &str| self.git(&pr.repo, &["rev-parse", what]).map(|t| t.trim().to_string());
        let commits = self.git(&pr.repo, &["log", "--format=%s", &format!("{base}..{branch}")])?.lines().map(str::to_string).collect();
        let mut linked_issues = Vec::new();
        if let Some(issue) = e.repo.get("issue").filter(|i| !i.is_null()) {
            if linked_issue_numbers(&summary.title, &body).contains(&1) {
                linked_issues.push(LinkedIssue { number: 1, title: s(issue, "title"), body: s(issue, "body"), url: format!("https://github.com/{}/{}/issues/1", pr.owner, pr.repo) });
            }
        }
        Ok(PrDetail {
            summary,
            body,
            head_sha: sha(&branch)?,
            base_sha: sha(&base)?,
            base_ref: base,
            head_ref: branch,
            default_branch: "main".into(),
            state: "open".into(),
            commits,
            linked_issues,
            ci: CiStatus::default(),
        })
    }

    async fn get_changes(&self, pr: &PrRef) -> Result<Vec<FileChange>, String> {
        let e = self.find(pr)?;
        let range = format!("{}...{}", s(e.pr, "base"), s(e.pr, "branch"));
        let names = self.git(&pr.repo, &["diff", "--name-status", "-M", &range])?;
        let counts = self.git(&pr.repo, &["diff", "--numstat", "-M", &range])?;
        let patches = patches(&self.git(&pr.repo, &["diff", "--no-color", "-M", &range])?);
        let head = s(e.pr, "branch");
        Ok(names
            .lines()
            .zip(counts.lines())
            .enumerate()
            .map(|(i, (name, count))| {
                let cols: Vec<&str> = name.split('\t').collect();
                let status = cols.first().copied().unwrap_or("M");
                let path = cols.last().copied().unwrap_or("").to_string();
                let mut nums = count.split('\t').map(|n| n.parse::<u32>().unwrap_or(0));
                FileChange {
                    pr: pr.clone(),
                    url: format!("https://github.com/{}/{}/blob/{head}/{path}", pr.owner, pr.repo),
                    previous_path: if status.starts_with('R') { cols.get(1).map(|p| p.to_string()) } else { None },
                    kind: match status.chars().next() {
                        Some('A') | Some('C') => ChangeKind::Added,
                        Some('D') => ChangeKind::Removed,
                        Some('R') => ChangeKind::Renamed,
                        _ => ChangeKind::Modified,
                    },
                    additions: nums.next().unwrap_or(0),
                    deletions: nums.next().unwrap_or(0),
                    patch: patches.get(i).cloned().flatten(),
                    hunks: Vec::new(),
                    trimmed: None,
                    path,
                }
            })
            .collect())
    }

    async fn get_file(&self, detail: &PrDetail, path: &str, at: At) -> Result<Option<String>, String> {
        let sha = if at == At::Base { &detail.base_sha } else { &detail.head_sha };
        Ok(self.git(&detail.summary.pr.repo, &["show", &format!("{sha}:{path}")]).ok())
    }

    async fn get_history(&self, detail: &PrDetail, paths: &[String], per_path: u32) -> Result<Vec<Vec<String>>, String> {
        let repo = &detail.summary.pr.repo;
        let mut shas: Vec<String> = Vec::new();
        for path in paths {
            for sha in self.git(repo, &["log", &format!("-n{per_path}"), "--format=%H", &detail.base_sha, "--", path]).unwrap_or_default().lines() {
                if !shas.iter().any(|s| s == sha) && shas.len() < 60 {
                    shas.push(sha.to_string());
                }
            }
        }
        let mut out = Vec::new();
        for sha in shas {
            let files: Vec<String> = self.git(repo, &["show", "--name-only", "--format=", &sha]).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
            if !files.is_empty() && files.len() <= 120 {
                out.push(files);
            }
        }
        Ok(out)
    }

    async fn list_threads(&self, _pr: &PrRef) -> Result<Vec<Thread>, String> {
        Ok(Vec::new())
    }

    async fn submit_review(&self, _review: &ReviewSubmission) -> Result<String, String> {
        Err("This is an offline folder. There is nowhere to post a review.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_diff_into_one_patch_per_file() {
        let diff = "diff --git a/a.ts b/a.ts\nindex 1..2 100644\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1,2 @@\n a\n+b\ndiff --git a/logo.png b/logo.png\nBinary files differ\ndiff --git a/c.ts b/c.ts\nnew file mode 100644\n--- /dev/null\n+++ b/c.ts\n@@ -0,0 +1 @@\n+c";
        assert_eq!(patches(diff), vec![Some("@@ -1 +1,2 @@\n a\n+b".to_string()), None, Some("@@ -0,0 +1 @@\n+c".to_string())]);
    }
}
