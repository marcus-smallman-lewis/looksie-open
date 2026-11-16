//! Looksie's data model. Everything the engine, the connectors and the app pass to each other.
//! Field names are camelCase on the wire so the TypeScript UI can use them as they are.

use serde::{Deserialize, Serialize};

/// Which pull request, on which host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PrRef {
    /// "github" in v1. Later: "azure-devops", "gitlab", "local".
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    pub fn github(owner: &str, repo: &str, number: u64) -> Self {
        Self { host: "github".into(), owner: owner.into(), repo: repo.into(), number }
    }
    /// "owner/repo #12"
    pub fn label(&self) -> String {
        format!("{}/{} #{}", self.owner, self.repo, self.number)
    }
    /// "repo #12"
    pub fn short(&self) -> String {
        format!("{} #{}", self.repo, self.number)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrSummary {
    pub pr: PrRef,
    pub title: String,
    pub author: String,
    /// Written by a bot or coding agent, as far as we can tell.
    pub agent: bool,
    /// The repository is private. Private repositories need Looksie Pro.
    pub private: bool,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub updated_at: String,
    pub url: String,
    /// "review-requested" or "mine"
    pub relation: String,
    /// Things this pull request explicitly points at: "issue:owner/repo#12", "ref:owner/repo#34", "key:ABC-123".
    /// Used to suggest groups. Never branch names.
    #[serde(default)]
    pub links: Vec<String>,
    /// The latest commit. Lets the app tell whether a review it prepared earlier is still current.
    #[serde(default)]
    pub head_sha: String,
}

/// Pull requests that look like one piece of work, and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSuggestion {
    pub prs: Vec<PrRef>,
    pub name: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CiStatus {
    pub total: u32,
    pub failed: u32,
    pub pending: u32,
    /// The host would not say (no permission, or an error). Not the same as "no CI ran".
    #[serde(default)]
    pub unknown: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetail {
    pub summary: PrSummary,
    pub body: String,
    pub head_sha: String,
    pub base_sha: String,
    pub base_ref: String,
    pub head_ref: String,
    pub default_branch: String,
    pub state: String,
    pub commits: Vec<String>,
    pub linked_issues: Vec<LinkedIssue>,
    pub ci: CiStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
    Renamed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LineKind {
    Add,
    Del,
    Ctx,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub pr: PrRef,
    pub path: String,
    pub previous_path: Option<String>,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    /// The raw unified diff for this file, when the host sent one. Large and binary files have none.
    pub patch: Option<String>,
    /// `patch`, parsed. Filled in by the engine.
    #[serde(default)]
    pub hunks: Vec<Hunk>,
    pub url: String,
    /// Set when the engine kept only part of this file's changes (generated files, or very long ones):
    /// how many changed lines there were in all. The rest is on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trimmed: Option<u32>,
}

/// A comment that is already on the pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub pr: PrRef,
    pub path: String,
    pub line: Option<u32>,
    pub author: String,
    pub body: String,
    pub url: String,
}

// ---------- what the engine produces ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    Data,
    Server,
    Screens,
    TestsAndBuild,
    Docs,
}

impl Layer {
    pub fn title(&self) -> &'static str {
        match self {
            Layer::Data => "Data",
            Layer::Server => "Server",
            Layer::Screens => "Screens",
            Layer::TestsAndBuild => "Tests and build",
            Layer::Docs => "Docs",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Flag {
    None,
    Look,
    Problem,
}

/// Something true about the change, and who found it ("rule" or "ai").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub text: String,
    pub source: String,
}

impl Fact {
    pub fn rule(text: impl Into<String>) -> Self {
        Self { text: text.into(), source: "rule".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepFile {
    pub pr: PrRef,
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    /// When set, only these line ranges of the file's new side matter for this step (a huge lockfile, say).
    pub focus: Option<Vec<String>>,
}

/// The AI layer for one step. Always optional; the step works without it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepAi {
    pub question: String,
    pub why: String,
    pub flag: Flag,
}

/// How much of the reviewer's attention a step should get.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Needs judgement. Gets a question.
    Decide,
    /// Routine. Acknowledged at a glance, a batch at a time.
    Glance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub title: String,
    pub layer: Layer,
    pub tier: Tier,
    /// How strongly this asks for attention. Higher first.
    pub score: u32,
    /// Why it is a decision, or why it is routine. Plain words, one line each.
    pub reasons: Vec<String>,
    pub files: Vec<StepFile>,
    pub facts: Vec<Fact>,
    /// A question a rule can ask on its own. None means "read the change and decide".
    pub question: Option<String>,
    pub flag: Flag,
    pub builds_on: Vec<String>,
    pub leads_to: Vec<String>,
    pub ai: Option<StepAi>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClaimStatus {
    Backed,
    Contradicted,
    NotFound,
    Unchecked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub pr: PrRef,
    pub text: String,
    pub status: ClaimStatus,
    pub note: String,
    pub step_id: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Described {
    pub pr: PrRef,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Briefing {
    pub title: String,
    /// For a group: one line on its size, how the pull requests connect, and which to merge first.
    #[serde(default)]
    pub headline: Option<String>,
    /// The authors' own descriptions, one per pull request.
    pub descriptions: Vec<Described>,
    /// Written by the AI layer, when it is on.
    pub ai_summary: Option<String>,
    pub claims: Vec<Claim>,
    pub needs_you: Vec<Fact>,
    pub not_mentioned: Vec<Fact>,
    /// How the pull requests in a group fit together.
    pub fits: Vec<Fact>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub pr: PrRef,
    pub path: String,
    pub reason: String,
    pub lines: u32,
}

/// One review: one pull request, or several that belong together.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: String,
    pub prs: Vec<PrDetail>,
    pub briefing: Briefing,
    pub steps: Vec<Step>,
    pub files: Vec<FileChange>,
    pub skipped: Vec<SkippedFile>,
    pub threads: Vec<Thread>,
}

// ---------- what the reviewer sends back ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Old,
    New,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub path: String,
    pub side: Side,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftComment {
    /// None means a comment on the pull request as a whole.
    pub anchor: Option<Anchor>,
    pub body: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSubmission {
    pub pr: PrRef,
    /// The commit the reviewer looked at. The host rejects the review if the pull request has moved on.
    pub commit: String,
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub comments: Vec<DraftComment>,
}

/// What a host can do. The app hides what it cannot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub inline_comments: bool,
    pub multi_line: bool,
    pub suggestions: bool,
    pub verdicts: bool,
    pub reply: bool,
}

// ---------- the AI layer: optional, and always kept apart from the facts ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStep {
    pub id: String,
    pub question: String,
    pub why: String,
    pub flag: Flag,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiClaim {
    /// Position of the claim in the briefing's list.
    pub index: usize,
    pub status: ClaimStatus,
    pub note: String,
}

/// What an AI added to a review, after the engine checked it. The review itself is never changed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiLayer {
    pub provider: String,
    pub summary: Option<String>,
    pub steps: Vec<AiStep>,
    pub claims: Vec<AiClaim>,
    /// Answers thrown away because they did not point at real lines in the diff.
    pub dropped: u32,
}
