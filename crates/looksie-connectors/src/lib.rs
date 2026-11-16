//! Every network call Looksie makes to a git host lives in this crate, so anyone can read exactly what is sent.

pub mod folder;
pub mod github;

use looksie_protocol::{Capabilities, FileChange, PrDetail, PrRef, PrSummary, ReviewSubmission, Thread};

/// Which side of a pull request a file is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    Base,
    Head,
}

/// One git host. Reading and writing both go through here, so a new host is one new implementation.
#[allow(async_fn_in_trait)]
pub trait Connector {
    fn kind(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// Open pull requests waiting on the signed-in user, and ones they (or their agent) opened.
    async fn list_my_prs(&self) -> Result<Vec<PrSummary>, String>;
    async fn get_pr(&self, pr: &PrRef) -> Result<PrDetail, String>;
    async fn get_changes(&self, pr: &PrRef) -> Result<Vec<FileChange>, String>;
    /// The whole file at the base or head of the pull request. None when it does not exist there.
    async fn get_file(&self, detail: &PrDetail, path: &str, at: At) -> Result<Option<String>, String>;
    /// For each of `paths`, the files changed by the last few commits that touched it, as of the pull request's base.
    /// One list of file paths per commit. Used to learn which files normally change together.
    async fn get_history(&self, detail: &PrDetail, paths: &[String], per_path: u32) -> Result<Vec<Vec<String>>, String>;
    /// Comments already on the pull request's lines.
    async fn list_threads(&self, pr: &PrRef) -> Result<Vec<Thread>, String>;
    /// Posts one review. Only ever called after the reviewer presses Submit.
    async fn submit_review(&self, review: &ReviewSubmission) -> Result<String, String>;
}
