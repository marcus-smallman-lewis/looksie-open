//! GitHub. Talks to api.github.com (or a GitHub Enterprise address) and nowhere else.

use crate::{At, Connector};
use looksie_protocol::*;
use serde_json::{json, Value};

pub struct GitHub {
    client: reqwest::Client,
    token: String,
    api: String,
    graphql: String,
}

/// Tidies what someone typed as a server address into a bare host name: "https://GitHub.Acme.com/" gives "github.acme.com".
/// None when it is not a host name at all.
pub fn clean_host(text: &str) -> Option<String> {
    let t = text.trim().to_lowercase();
    let t = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).unwrap_or(&t);
    let host = t.split('/').next().unwrap_or("").trim_start_matches("www.");
    if host.is_empty() {
        return Some("github.com".into());
    }
    let ok = host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':');
    ok.then(|| host.to_string())
}

/// Where a GitHub server's two APIs live: (REST, GraphQL).
/// github.com, a company's own server (GitHub Enterprise Server), and GitHub's hosted company servers (*.ghe.com) each differ.
pub fn api_addresses(host: &str) -> (String, String) {
    if host == "github.com" {
        ("https://api.github.com".into(), "https://api.github.com/graphql".into())
    } else if host.ends_with(".ghe.com") {
        (format!("https://api.{host}"), format!("https://api.{host}/graphql"))
    } else {
        (format!("https://{host}/api/v3"), format!("https://{host}/api/graphql"))
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn n(v: &Value, key: &str) -> u32 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0) as u32
}

/// Percent-encodes a repository path, keeping the slashes.
fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Bots and coding agents, as far as the author name and description show.
pub fn looks_like_agent(author: &str, body: &str) -> bool {
    let a = author.to_lowercase();
    a.ends_with("[bot]")
        || ["dependabot", "renovate", "copilot-swe-agent", "copilot", "devin-ai-integration", "cursoragent", "codex"].iter().any(|b| a == *b || a.starts_with(&format!("{b}[")))
        || body.contains("Generated with [Claude Code]")
        || body.contains("Co-Authored-By: Claude")
        || body.contains("Generated with Claude Code")
        || body.contains("🤖 Generated with")
}

/// The issue numbers a description says it closes, plus "[#12]" in the title.
pub fn linked_issue_numbers(title: &str, body: &str) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let lower = body.to_lowercase();
    for word in ["closes", "close", "closed", "fixes", "fix", "fixed", "resolves", "resolve", "resolved"] {
        let mut rest = lower.as_str();
        while let Some(i) = rest.find(word) {
            let after = rest[i + word.len()..].trim_start_matches([' ', ':']);
            if let Some(num) = after.strip_prefix('#') {
                let digits: String = num.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(v) = digits.parse() {
                    out.push(v);
                }
            }
            rest = &rest[i + word.len()..];
        }
    }
    let mut t = title;
    while let Some(i) = t.find("[#") {
        let digits: String = t[i + 2..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = digits.parse() {
            out.push(v);
        }
        t = &t[i + 2..];
    }
    out.sort();
    out.dedup();
    out
}

/// Everything a pull request explicitly points at: issues it closes, other pull requests or issues it names, tracker keys.
pub fn links_of(pr: &PrRef, title: &str, body: &str) -> Vec<String> {
    let mut out: Vec<String> = linked_issue_numbers(title, body).into_iter().map(|n| format!("issue:{}/{}#{n}", pr.owner, pr.repo)).collect();
    let cross = regex::Regex::new(r"([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)#(\d+)").unwrap();
    for c in cross.captures_iter(body) {
        out.push(format!("ref:{}/{}#{}", &c[1], &c[2], &c[3]));
    }
    let url = regex::Regex::new(r"github.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/(?:pull|issues)/(\d+)").unwrap();
    for c in url.captures_iter(body) {
        out.push(format!("ref:{}/{}#{}", &c[1], &c[2], &c[3]));
    }
    let tracker = regex::Regex::new(r"\b([A-Z][A-Z0-9]{1,9}-\d{1,6})\b").unwrap();
    // Tracker keys are taken from the title only, and things that merely look like one are ignored.
    const NOT_KEYS: &[&str] = &["UTF", "SHA", "ISO", "RFC", "CVE", "HTTP", "TLS", "SSL", "AES", "RSA", "MD", "ES", "PEP", "COVID", "GPT", "X86", "ARM", "UTF8", "BASE"];
    for c in tracker.captures_iter(title) {
        let prefix = c[1].split('-').next().unwrap_or("");
        if !NOT_KEYS.contains(&prefix) {
            out.push(format!("key:{}", &c[1]));
        }
    }
    let me = format!("ref:{}/{}#{}", pr.owner, pr.repo, pr.number);
    out.retain(|l| l != &me);
    out.sort();
    out.dedup();
    out
}

/// The exact JSON sent to GitHub for a review. Public so the app can show it before anything is posted.
/// The most pull requests the inbox asks for, per list (waiting on you, opened by you).
pub const INBOX_MAX: usize = 500;

/// One pull request from a GraphQL search, as an inbox row. None for anything that is not a pull request.
pub fn summary_from_node(node: &Value, relation: &str) -> Option<PrSummary> {
    let number = node.get("number").and_then(Value::as_u64)?;
    let owner = node.pointer("/repository/owner/login").and_then(Value::as_str)?;
    let repo = node.pointer("/repository/name").and_then(Value::as_str)?;
    let pr = PrRef::github(owner, repo, number);
    let (title, body) = (s(node, "title"), s(node, "body"));
    // The other API calls an app "dependabot[bot]"; this one says "dependabot" and marks it as a Bot.
    let mut author = node.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_string();
    if node.pointer("/author/__typename").and_then(Value::as_str) == Some("Bot") && !author.ends_with("[bot]") {
        author.push_str("[bot]");
    }
    Some(PrSummary {
        agent: looks_like_agent(&author, &body),
        author,
        private: node.pointer("/repository/isPrivate").and_then(Value::as_bool).unwrap_or(false),
        additions: n(node, "additions"),
        deletions: n(node, "deletions"),
        changed_files: n(node, "changedFiles"),
        updated_at: s(node, "updatedAt"),
        url: s(node, "url"),
        relation: relation.to_string(),
        links: links_of(&pr, &title, &body),
        head_sha: s(node, "headRefOid"),
        title,
        pr,
    })
}

pub fn review_payload(review: &ReviewSubmission) -> Value {
    let mut body = review.summary.trim().to_string();
    let mut comments = Vec::new();
    for c in &review.comments {
        match &c.anchor {
            None => {
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str(c.body.trim());
            }
            Some(a) => {
                let side = if a.side == Side::Old { "LEFT" } else { "RIGHT" };
                let mut o = json!({ "path": a.path, "line": a.end_line, "side": side, "body": c.body });
                if a.start_line < a.end_line {
                    o["start_line"] = json!(a.start_line);
                    o["start_side"] = json!(side);
                }
                comments.push(o);
            }
        }
    }
    let event = match review.verdict {
        ReviewVerdict::Approve => "APPROVE",
        ReviewVerdict::RequestChanges => "REQUEST_CHANGES",
        ReviewVerdict::Comment => "COMMENT",
    };
    if body.is_empty() && review.verdict == ReviewVerdict::RequestChanges {
        body = "Changes requested. See the comments on the lines.".to_string();
    }
    json!({ "commit_id": review.commit, "event": event, "body": body, "comments": comments })
}

impl GitHub {
    /// github.com.
    pub fn new(token: String) -> Result<Self, String> {
        Self::for_host(token, "github.com")
    }

    /// Any GitHub server, by host name. The token is only ever sent to that host.
    pub fn for_host(token: String, host: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder().user_agent("looksie").build().map_err(|e| e.to_string())?;
        let (api, graphql) = api_addresses(host);
        Ok(Self { client, token, api, graphql })
    }

    /// One GraphQL query. Used for the inbox, where it gets a hundred pull requests in one request.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        let res = self
            .client
            .post(&self.graphql)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {e}"))?;
        let status = res.status();
        let v: Value = res.json().await.map_err(|e| format!("GitHub sent something unreadable: {e}"))?;
        if !status.is_success() {
            return Err(format!("GitHub said {status}: {}", s(&v, "message")));
        }
        if let Some(first) = v.get("errors").and_then(Value::as_array).and_then(|e| e.first()) {
            if v.get("data").map(Value::is_null).unwrap_or(true) {
                return Err(format!("GitHub could not answer: {}", s(first, "message")));
            }
        }
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    /// Open pull requests matching a search, newest first, up to `max` of them. A hundred per request.
    async fn search_fast(&self, query: &str, relation: &str, max: usize) -> Result<Vec<PrSummary>, String> {
        const QUERY: &str = "query($q: String!, $after: String) { search(query: $q, type: ISSUE, first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { ... on PullRequest { number title body url updatedAt additions deletions changedFiles headRefOid author { __typename login } repository { name isPrivate owner { login } } } } } }";
        let mut out = Vec::new();
        let mut after = Value::Null;
        while out.len() < max {
            let data = self.graphql(QUERY, json!({ "q": query, "after": after })).await?;
            let search = data.get("search").cloned().unwrap_or(Value::Null);
            for node in search.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default() {
                if let Some(p) = summary_from_node(&node, relation) {
                    out.push(p);
                }
            }
            if !search.pointer("/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false) {
                break;
            }
            after = search.pointer("/pageInfo/endCursor").cloned().unwrap_or(Value::Null);
        }
        out.truncate(max);
        Ok(out)
    }

    async fn send(&self, url: &str, accept: &str) -> Result<reqwest::Response, String> {
        self.client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {e}"))
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{}", self.api, path);
        let res = self.send(&url, "application/vnd.github+json").await?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text).ok().map(|v| s(&v, "message")).filter(|m| !m.is_empty()).unwrap_or(text);
            return Err(format!("GitHub said {status} for {path}: {msg}"));
        }
        res.json().await.map_err(|e| format!("GitHub sent something unreadable for {path}: {e}"))
    }

    async fn pages(&self, path: &str, max_pages: u32) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for page in 1..=max_pages {
            let sep = if path.contains('?') { '&' } else { '?' };
            let v = self.get(&format!("{path}{sep}per_page=100&page={page}")).await?;
            let items = v.as_array().cloned().unwrap_or_default();
            let count = items.len();
            out.extend(items);
            if count < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// The signed-in user's login name.
    pub async fn viewer(&self) -> Result<String, String> {
        Ok(s(&self.get("/user").await?, "login"))
    }

    fn summary_from(pr: &PrRef, v: &Value, relation: &str) -> PrSummary {
        let author = v.get("user").map(|u| s(u, "login")).unwrap_or_default();
        let body = s(v, "body");
        PrSummary {
            pr: pr.clone(),
            title: s(v, "title"),
            agent: looks_like_agent(&author, &body),
            author,
            private: v.pointer("/base/repo/private").and_then(Value::as_bool).unwrap_or(false),
            additions: n(v, "additions"),
            deletions: n(v, "deletions"),
            changed_files: n(v, "changed_files"),
            updated_at: s(v, "updated_at"),
            url: s(v, "html_url"),
            relation: relation.to_string(),
            links: links_of(pr, &s(v, "title"), &body),
            head_sha: v.pointer("/head/sha").and_then(Value::as_str).unwrap_or("").to_string(),
        }
    }

    async fn search(&self, query: &str, relation: &str, out: &mut Vec<PrSummary>) -> Result<(), String> {
        let v = self.get(&format!("/search/issues?q={}&per_page=100&sort=updated", query.replace(' ', "+"))).await?;
        let mut wanted: Vec<PrRef> = Vec::new();
        for item in v.get("items").and_then(Value::as_array).cloned().unwrap_or_default() {
            let repo_url = s(&item, "repository_url");
            let mut parts = repo_url.rsplit('/');
            let (repo, owner) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let pr = PrRef::github(owner, repo, item.get("number").and_then(Value::as_u64).unwrap_or(0));
            if !out.iter().any(|x| x.pr == pr) && !wanted.contains(&pr) {
                wanted.push(pr);
            }
        }
        // The search result has no size or privacy, so ask for each pull request itself, all at once.
        let paths: Vec<String> = wanted.iter().map(|pr| format!("/repos/{}/{}/pulls/{}", pr.owner, pr.repo, pr.number)).collect();
        let fetched = futures_util::future::join_all(paths.iter().map(|p| self.get(p))).await;
        for (pr, full) in wanted.iter().zip(fetched) {
            // One pull request that cannot be read (deleted, access removed) should not empty the whole inbox.
            if let Ok(full) = full {
                out.push(Self::summary_from(pr, &full, relation));
            }
        }
        Ok(())
    }
}

impl Connector for GitHub {
    fn kind(&self) -> &'static str {
        "github"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { inline_comments: true, multi_line: true, suggestions: true, verdicts: true, reply: true }
    }

    async fn list_my_prs(&self) -> Result<Vec<PrSummary>, String> {
        // The fast way: two searches at once, a hundred pull requests per request, sizes included.
        let me = self.viewer().await?;
        let waiting = format!("is:pr is:open archived:false review-requested:{me} sort:updated-desc");
        let mine = format!("is:pr is:open archived:false author:{me} sort:updated-desc");
        let (a, b) = futures_util::future::join(self.search_fast(&waiting, "review-requested", INBOX_MAX), self.search_fast(&mine, "mine", INBOX_MAX)).await;
        if let (Ok(mut out), Ok(b)) = (a, b) {
            for p in b {
                if !out.iter().any(|x| x.pr == p.pr) {
                    out.push(p);
                }
            }
            return Ok(out);
        }
        // An older server, or a token that may not use that API: ask the slower way.
        let mut out = Vec::new();
        self.search("is:pr is:open review-requested:@me archived:false", "review-requested", &mut out).await?;
        self.search("is:pr is:open author:@me archived:false", "mine", &mut out).await?;
        Ok(out)
    }

    async fn get_pr(&self, pr: &PrRef) -> Result<PrDetail, String> {
        let base = format!("/repos/{}/{}", pr.owner, pr.repo);
        let v = self.get(&format!("{base}/pulls/{}", pr.number)).await?;
        let summary = Self::summary_from(pr, &v, "");
        let head_sha = v.pointer("/head/sha").and_then(Value::as_str).unwrap_or("").to_string();
        let body = s(&v, "body");

        let commits = self.pages(&format!("{base}/pulls/{}/commits", pr.number), 3).await?;
        let commits = commits.iter().map(|c| c.pointer("/commit/message").and_then(Value::as_str).unwrap_or("").lines().next().unwrap_or("").to_string()).collect();

        let mut linked_issues = Vec::new();
        for num in linked_issue_numbers(&summary.title, &body).into_iter().take(3) {
            if let Ok(i) = self.get(&format!("{base}/issues/{num}")).await {
                if i.get("pull_request").is_none() {
                    linked_issues.push(LinkedIssue { number: num, title: s(&i, "title"), body: s(&i, "body"), url: s(&i, "html_url") });
                }
            }
        }

        let mut ci = CiStatus::default();
        let runs = self.get(&format!("{base}/commits/{head_sha}/check-runs?per_page=100")).await;
        ci.unknown = runs.is_err();
        if let Ok(runs) = runs {
            for r in runs.get("check_runs").and_then(Value::as_array).cloned().unwrap_or_default() {
                ci.total += 1;
                if s(&r, "status") != "completed" {
                    ci.pending += 1;
                } else if matches!(s(&r, "conclusion").as_str(), "failure" | "timed_out" | "cancelled" | "action_required") {
                    ci.failed += 1;
                }
            }
        }
        if let Ok(st) = self.get(&format!("{base}/commits/{head_sha}/status")).await {
            for r in st.get("statuses").and_then(Value::as_array).cloned().unwrap_or_default() {
                ci.total += 1;
                match s(&r, "state").as_str() {
                    "pending" => ci.pending += 1,
                    "failure" | "error" => ci.failed += 1,
                    _ => {}
                }
            }
        }

        Ok(PrDetail {
            summary,
            body,
            head_sha,
            base_sha: v.pointer("/base/sha").and_then(Value::as_str).unwrap_or("").to_string(),
            base_ref: v.pointer("/base/ref").and_then(Value::as_str).unwrap_or("").to_string(),
            head_ref: v.pointer("/head/ref").and_then(Value::as_str).unwrap_or("").to_string(),
            default_branch: v.pointer("/base/repo/default_branch").and_then(Value::as_str).unwrap_or("").to_string(),
            state: if v.get("merged").and_then(Value::as_bool).unwrap_or(false) { "merged".into() } else { s(&v, "state") },
            commits,
            linked_issues,
            ci,
        })
    }

    async fn get_changes(&self, pr: &PrRef) -> Result<Vec<FileChange>, String> {
        let files = self.pages(&format!("/repos/{}/{}/pulls/{}/files", pr.owner, pr.repo, pr.number), 30).await?;
        Ok(files
            .iter()
            .map(|f| FileChange {
                pr: pr.clone(),
                path: s(f, "filename"),
                previous_path: f.get("previous_filename").and_then(Value::as_str).map(str::to_string),
                kind: match s(f, "status").as_str() {
                    "added" | "copied" => ChangeKind::Added,
                    "removed" => ChangeKind::Removed,
                    "renamed" => ChangeKind::Renamed,
                    _ => ChangeKind::Modified,
                },
                additions: n(f, "additions"),
                deletions: n(f, "deletions"),
                patch: f.get("patch").and_then(Value::as_str).map(str::to_string),
                hunks: Vec::new(),
                url: s(f, "blob_url"),
                trimmed: None,
            })
            .collect())
    }

    async fn get_file(&self, detail: &PrDetail, path: &str, at: At) -> Result<Option<String>, String> {
        let pr = &detail.summary.pr;
        let sha = if at == At::Base { &detail.base_sha } else { &detail.head_sha };
        let url = format!("{}/repos/{}/{}/contents/{}?ref={}", self.api, pr.owner, pr.repo, encode_path(path), sha);
        let res = self.send(&url, "application/vnd.github.raw+json").await?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(format!("GitHub said {} for {path}", res.status()));
        }
        res.text().await.map(Some).map_err(|e| e.to_string())
    }

    async fn get_history(&self, detail: &PrDetail, paths: &[String], per_path: u32) -> Result<Vec<Vec<String>>, String> {
        let pr = &detail.summary.pr;
        let base = format!("/repos/{}/{}", pr.owner, pr.repo);
        let lists: Vec<String> = paths.iter().map(|p| format!("{base}/commits?sha={}&path={}&per_page={per_path}", detail.base_sha, encode_path(p))).collect();
        let mut shas: Vec<String> = Vec::new();
        for found in futures_util::future::join_all(lists.iter().map(|u| self.get(u))).await.into_iter().flatten() {
            for c in found.as_array().cloned().unwrap_or_default() {
                let sha = s(&c, "sha");
                if !sha.is_empty() && !shas.contains(&sha) && shas.len() < 60 {
                    shas.push(sha);
                }
            }
        }
        let details: Vec<String> = shas.iter().map(|sha| format!("{base}/commits/{sha}")).collect();
        let mut out = Vec::new();
        for c in futures_util::future::join_all(details.iter().map(|u| self.get(u))).await.into_iter().flatten() {
            let files: Vec<String> = c.get("files").and_then(Value::as_array).map(|a| a.iter().map(|f| s(f, "filename")).collect()).unwrap_or_default();
            // A commit touching hundreds of files (a mass rename, a merge) says nothing about what belongs together.
            if !files.is_empty() && files.len() <= 120 {
                out.push(files);
            }
        }
        Ok(out)
    }

    async fn list_threads(&self, pr: &PrRef) -> Result<Vec<Thread>, String> {
        let items = self.pages(&format!("/repos/{}/{}/pulls/{}/comments", pr.owner, pr.repo, pr.number), 5).await?;
        Ok(items
            .iter()
            .map(|c| Thread {
                pr: pr.clone(),
                path: s(c, "path"),
                line: c.get("line").and_then(Value::as_u64).or_else(|| c.get("original_line").and_then(Value::as_u64)).map(|l| l as u32),
                author: c.get("user").map(|u| s(u, "login")).unwrap_or_default(),
                body: s(c, "body"),
                url: s(c, "html_url"),
            })
            .collect())
    }

    async fn submit_review(&self, review: &ReviewSubmission) -> Result<String, String> {
        let pr = &review.pr;
        let url = format!("{}/repos/{}/{}/pulls/{}/reviews", self.api, pr.owner, pr.repo, pr.number);
        let res = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&review_payload(review))
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {e}"))?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!("GitHub would not accept the review ({status}): {}", s(&v, "message")));
        }
        Ok(s(&v, "html_url"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_linked_issues_in_titles_and_bodies() {
        assert_eq!(linked_issue_numbers("[#648] Patch npm audit advisories", "Closes #648.\n\nFixes: #12 and resolves #7"), vec![7, 12, 648]);
        assert!(linked_issue_numbers("Docs", "See PR #5").is_empty());
    }

    #[test]
    fn collects_explicit_links_only() {
        let pr = PrRef::github("northwind", "mobile", 88);
        let links = links_of(&pr, "[#398] Profile toggle (ARC-142)", "Closes #398. Works with northwind/arcade#412, see https://github.com/northwind/admin/pull/54.");
        assert_eq!(links, vec!["issue:northwind/mobile#398", "key:ARC-142", "ref:northwind/admin#54", "ref:northwind/arcade#412"]);
        assert!(links_of(&pr, "Tidy up", "Nothing to see. UTF-8 only.").is_empty());
    }

    #[test]
    fn spots_agents() {
        assert!(looks_like_agent("dependabot[bot]", ""));
        assert!(looks_like_agent("marcus", "Does a thing.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)"));
        assert!(!looks_like_agent("marcus", "I wrote this by hand."));
    }

    #[test]
    fn review_payload_matches_what_github_expects() {
        let review = ReviewSubmission {
            pr: PrRef::github("o", "r", 1),
            commit: "abc".into(),
            verdict: ReviewVerdict::RequestChanges,
            summary: "Two things.".into(),
            comments: vec![
                DraftComment { anchor: Some(Anchor { path: "a.ts".into(), side: Side::New, start_line: 10, end_line: 12 }), body: "Why?".into() },
                DraftComment { anchor: Some(Anchor { path: "b.ts".into(), side: Side::Old, start_line: 4, end_line: 4 }), body: "Keep this.".into() },
                DraftComment { anchor: None, body: "Overall: nearly there.".into() },
            ],
        };
        let p = review_payload(&review);
        assert_eq!(p["event"], "REQUEST_CHANGES");
        assert_eq!(p["commit_id"], "abc");
        assert_eq!(p["body"], "Two things.\n\nOverall: nearly there.");
        let bare = ReviewSubmission { summary: String::new(), comments: vec![], ..review.clone() };
        assert_eq!(review_payload(&bare)["body"], "Changes requested. See the comments on the lines.");
        assert_eq!(p["comments"][0], json!({ "path": "a.ts", "line": 12, "side": "RIGHT", "start_line": 10, "start_side": "RIGHT", "body": "Why?" }));
        assert_eq!(p["comments"][1], json!({ "path": "b.ts", "line": 4, "side": "LEFT", "body": "Keep this." }));
    }

    #[test]
    fn knows_where_each_kind_of_github_server_keeps_its_apis() {
        assert_eq!(api_addresses("github.com"), ("https://api.github.com".to_string(), "https://api.github.com/graphql".to_string()));
        assert_eq!(api_addresses("github.acme.com"), ("https://github.acme.com/api/v3".to_string(), "https://github.acme.com/api/graphql".to_string()));
        assert_eq!(api_addresses("acme.ghe.com"), ("https://api.acme.ghe.com".to_string(), "https://api.acme.ghe.com/graphql".to_string()));
    }

    #[test]
    fn tidies_a_typed_server_address() {
        assert_eq!(clean_host("").as_deref(), Some("github.com"));
        assert_eq!(clean_host(" https://GitHub.Acme.com/login ").as_deref(), Some("github.acme.com"));
        assert_eq!(clean_host("github.acme.com:8443").as_deref(), Some("github.acme.com:8443"));
        assert_eq!(clean_host("not a server"), None);
        assert_eq!(clean_host("localhost"), None);
    }

    #[test]
    fn reads_an_inbox_row_from_a_graphql_search() {
        let node = json!({
            "number": 7, "title": "Bump express", "body": "Closes #3", "url": "https://github.com/acme/api/pull/7", "updatedAt": "2026-11-01T09:00:00Z",
            "additions": 14, "deletions": 14, "changedFiles": 2,
            "author": { "__typename": "Bot", "login": "dependabot" },
            "repository": { "name": "api", "isPrivate": true, "owner": { "login": "acme" } }
        });
        let p = summary_from_node(&node, "review-requested").unwrap();
        assert_eq!(p.pr, PrRef::github("acme", "api", 7));
        assert_eq!((p.author.as_str(), p.agent, p.private, p.additions, p.changed_files), ("dependabot[bot]", true, true, 14, 2));
        assert_eq!(p.links, vec!["issue:acme/api#3"]);
        assert!(summary_from_node(&json!({}), "mine").is_none(), "an issue, or something we cannot see, is skipped");
    }

    #[test]
    fn encodes_awkward_paths() {
        assert_eq!(encode_path("mobile/app/(tabs)/[id].tsx"), "mobile/app/%28tabs%29/%5Bid%5D.tsx");
    }
}
