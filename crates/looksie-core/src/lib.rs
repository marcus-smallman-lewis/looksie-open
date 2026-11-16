//! Fetches everything a review needs through a connector, then asks the engine to build it.
//! Also: where AI answers are kept (`ai_store`), and the MCP server that lets the reviewer's own agent use Looksie (`mcp`).

pub mod ai_store;
pub mod mcp;

use looksie_connectors::{At, Connector};
use looksie_engine::{build_review, wanted_history_paths, wanted_lockfiles, LockPair, PrInput, ReviewInput};
use looksie_protocol::{PrRef, Review};

/// Parses "owner/repo#12", or a pull request's web address on github.com or a company's own GitHub server.
pub fn parse_pr(text: &str) -> Result<PrRef, String> {
    let t = text.trim();
    // "https://any.host/owner/repo/pull/12": drop the scheme and the host.
    let t = match t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")) {
        Some(rest) => rest.split_once('/').map(|x| x.1).unwrap_or(""),
        None => t,
    };
    let t = t.replace("/pull/", "#");
    let t = t.split(['?', ' ']).next().unwrap_or("").trim_end_matches("/files").trim_end_matches("/commits").to_string();
    let bad = || format!("'{text}' is not owner/repo#number");
    let (repo, number) = t.split_once('#').ok_or_else(bad)?;
    let (owner, repo) = repo.split_once('/').ok_or_else(bad)?;
    let number = number.trim_end_matches('/').parse().map_err(|_| bad())?;
    Ok(PrRef::github(owner, repo, number))
}

pub async fn load_pr<C: Connector>(host: &C, pr: &PrRef, handover: Option<String>) -> Result<PrInput, String> {
    let (detail, files, threads) = futures_util::future::join3(host.get_pr(pr), host.get_changes(pr), host.list_threads(pr)).await;
    let (detail, files, threads) = (detail?, files?, threads.unwrap_or_default());
    let mut lockfiles = Vec::new();
    for path in wanted_lockfiles(&files) {
        let base = host.get_file(&detail, &path, At::Base).await.unwrap_or(None);
        let head = host.get_file(&detail, &path, At::Head).await.unwrap_or(None);
        lockfiles.push(LockPair { path, base, head });
    }
    // History is a nice-to-have: if it cannot be read, the review still works without the paired-files rule.
    let history = host.get_history(&detail, &wanted_history_paths(&files), 12).await.unwrap_or_default();
    Ok(PrInput { detail, files, threads, lockfiles, handover, history })
}

/// One review from one or more pull requests.
pub async fn load_review<C: Connector>(host: &C, prs: &[PrRef], group_name: Option<String>) -> Result<Review, String> {
    if prs.is_empty() {
        return Err("Pick at least one pull request.".into());
    }
    // All at once: a group of eight would otherwise wait for eight round trips one after another.
    let inputs = futures_util::future::join_all(prs.iter().map(|pr| load_pr(host, pr, None))).await.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(build_review(ReviewInput { prs: inputs, group_name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pull_request_references() {
        assert_eq!(parse_pr("northwind/arcade#412").unwrap(), PrRef::github("northwind", "arcade", 412));
        assert_eq!(parse_pr("https://github.com/northwind/arcade/pull/412").unwrap(), PrRef::github("northwind", "arcade", 412));
        assert_eq!(parse_pr("https://github.acme.com/northwind/arcade/pull/412/files").unwrap(), PrRef::github("northwind", "arcade", 412));
        assert!(parse_pr("arcade 412").is_err());
    }
}

/// Pulls the first JSON object out of a reply, whether or not it is wrapped in a code fence.
pub fn extract_json(text: &str) -> Result<serde_json::Value, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { return Err("That is not a JSON object.".into()) };
    serde_json::from_str(&text[a..=b]).map_err(|e| format!("That is not valid JSON: {e}"))
}
