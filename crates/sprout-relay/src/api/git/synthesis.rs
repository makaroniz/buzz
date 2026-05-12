//! Git browse event synthesis — synthesize ephemeral Nostr events from git data.
//!
//! Called from the bridge query path. On any git error, returns `None` to fall
//! through to the normal DB query (which returns empty for ephemeral kinds).

use serde_json::Value;
use tracing::{debug, warn};

use sprout_core::kind::{KIND_GIT_BLOB, KIND_GIT_COMMIT_LOG, KIND_GIT_README, KIND_GIT_TREE};

use crate::state::AppState;

use super::browse::{
    find_readme, parse_commit_log_output, parse_tree_output, resolve_blob_metadata, resolve_repo,
    run_git, validate_ref, validate_tree_path, verify_ref, COMMITS_PER_PAGE,
};

const GIT_BROWSE_KINDS: &[u32] = &[
    KIND_GIT_TREE,
    KIND_GIT_BLOB,
    KIND_GIT_COMMIT_LOG,
    KIND_GIT_README,
];

/// Check if all filters target git browse kinds. If so, synthesize events.
/// Returns `Some(events)` if handled, `None` to fall through to normal query.
pub(crate) async fn synthesize_git_browse(
    state: &AppState,
    filters: &[nostr::Filter],
) -> Option<Vec<Value>> {
    // Only intercept if every filter targets exactly one git browse kind.
    let mut results: Vec<Value> = Vec::new();

    for filter in filters {
        let kinds = match filter.kinds.as_ref() {
            Some(k) => k,
            None => {
                debug!("git browse synthesis: filter has no kinds, skipping");
                return None;
            }
        };
        if kinds.len() != 1 {
            debug!(
                len = kinds.len(),
                "git browse synthesis: filter has != 1 kind, skipping"
            );
            return None;
        }
        let kind_val = match kinds.iter().next() {
            Some(k) => k,
            None => {
                debug!("git browse synthesis: kinds set empty after len check");
                return None;
            }
        };
        let kind = kind_val.as_u16() as u32;
        if !GIT_BROWSE_KINDS.contains(&kind) {
            debug!(
                kind,
                "git browse synthesis: kind not a git browse kind, skipping"
            );
            return None;
        }

        debug!(kind, "git browse synthesis: intercepted filter");

        // Extract required tags: #d = "owner/repo", #r = "ref"
        let d_values = match filter
            .generic_tags
            .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::D))
        {
            Some(v) => v,
            None => {
                warn!(kind, "git browse synthesis: filter missing #d tag");
                return None;
            }
        };
        let d_tag = match d_values.iter().next() {
            Some(v) => v.as_str(),
            None => {
                warn!(kind, "git browse synthesis: #d tag set is empty");
                return None;
            }
        };

        let r_values = match filter
            .generic_tags
            .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::R))
        {
            Some(v) => v,
            None => {
                warn!(kind, d_tag, "git browse synthesis: filter missing #r tag");
                return None;
            }
        };
        let git_ref = match r_values.iter().next() {
            Some(v) => v.as_str(),
            None => {
                warn!(kind, d_tag, "git browse synthesis: #r tag set is empty");
                return None;
            }
        };

        debug!(kind, d_tag, git_ref, "git browse synthesis: extracted tags");

        // Parse owner/repo from d-tag
        let (owner, repo) = match d_tag.split_once('/') {
            Some(pair) => pair,
            None => {
                warn!(d_tag, "git browse synthesis: d-tag missing '/' separator");
                return None;
            }
        };

        // Validate inputs — on failure, return None (fall through)
        if validate_ref(git_ref).is_err() {
            warn!(git_ref, "git browse synthesis: invalid ref");
            return None;
        }

        let repo_path = match resolve_repo(owner, repo, state) {
            Ok(path) => {
                debug!(?path, "git browse synthesis: resolved repo");
                path
            }
            Err(_) => {
                warn!(owner, repo, "git browse synthesis: resolve_repo failed");
                return None;
            }
        };
        if verify_ref(state, &repo_path, git_ref).await.is_err() {
            warn!(
                git_ref,
                ?repo_path,
                "git browse synthesis: ref not found in repo"
            );
            return None;
        }

        let event = match kind {
            KIND_GIT_TREE => {
                // Optional #f tag for path
                let path = filter
                    .generic_tags
                    .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::F))
                    .and_then(|vs| vs.iter().next())
                    .map(|s| s.as_str())
                    .unwrap_or("");

                if validate_tree_path(path).is_err() {
                    warn!(path, "git browse synthesis: invalid tree path");
                    return None;
                }

                let target = if path.is_empty() {
                    git_ref.to_string()
                } else {
                    let clean = path.strip_suffix('/').unwrap_or(path);
                    format!("{git_ref}:{clean}")
                };

                let stdout = match run_git(state, &repo_path, &["ls-tree", "--long", &target]).await
                {
                    Ok(out) => out,
                    Err(_) => {
                        warn!(?repo_path, target, "git browse synthesis: ls-tree failed");
                        return None;
                    }
                };
                let entries = parse_tree_output(&stdout);
                let content = match serde_json::to_string(&entries) {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(%e, "git browse synthesis: failed to serialize tree entries");
                        return None;
                    }
                };

                let mut tags = vec![
                    nostr::Tag::parse(&["d", d_tag]).ok()?,
                    nostr::Tag::parse(&["r", git_ref]).ok()?,
                ];
                if !path.is_empty() {
                    tags.push(nostr::Tag::parse(&["f", path]).ok()?);
                }

                match build_signed_event(state, KIND_GIT_TREE, &content, tags) {
                    Some(ev) => ev,
                    None => {
                        warn!("git browse synthesis: failed to build signed tree event");
                        return None;
                    }
                }
            }
            KIND_GIT_BLOB => {
                // Required #f tag for file path
                let path = match filter
                    .generic_tags
                    .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::F))
                    .and_then(|vs| vs.iter().next())
                    .map(|s| s.as_str())
                {
                    Some(p) => p,
                    None => {
                        warn!("git browse synthesis: blob filter missing #f tag");
                        return None;
                    }
                };

                if path.is_empty() || validate_tree_path(path).is_err() {
                    warn!(path, "git browse synthesis: invalid blob path");
                    return None;
                }
                let clean_path = path.strip_suffix('/').unwrap_or(path);

                let meta = match resolve_blob_metadata(state, &repo_path, git_ref, clean_path).await
                {
                    Ok(m) => m,
                    Err(_) => {
                        warn!(
                            clean_path,
                            "git browse synthesis: resolve_blob_metadata failed"
                        );
                        return None;
                    }
                };

                let encoded_ref = percent_encoding::utf8_percent_encode(
                    git_ref,
                    percent_encoding::NON_ALPHANUMERIC,
                );
                let blob_url = format!("/api/repos/{owner}/{repo}/blob/{encoded_ref}/{clean_path}");

                let tags = vec![
                    nostr::Tag::parse(&["d", d_tag]).ok()?,
                    nostr::Tag::parse(&["r", git_ref]).ok()?,
                    nostr::Tag::parse(&["f", path]).ok()?,
                    nostr::Tag::parse(&["size", &meta.size.to_string()]).ok()?,
                    nostr::Tag::parse(&["binary", if meta.is_binary { "true" } else { "false" }])
                        .ok()?,
                    nostr::Tag::parse(&["url", &blob_url]).ok()?,
                ];

                // Content is empty — actual file content served by REST blob endpoint
                match build_signed_event(state, KIND_GIT_BLOB, "", tags) {
                    Some(ev) => ev,
                    None => {
                        warn!("git browse synthesis: failed to build signed blob event");
                        return None;
                    }
                }
            }
            KIND_GIT_COMMIT_LOG => {
                // Optional #n tag for page number (avoids #p which collides with pubkey refs)
                let page: u32 = filter
                    .generic_tags
                    .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::N))
                    .and_then(|vs| vs.iter().next())
                    .and_then(|s| s.as_str().parse().ok())
                    .unwrap_or(0);

                let skip = page * COMMITS_PER_PAGE;
                let limit_str = COMMITS_PER_PAGE.to_string();
                let skip_str = skip.to_string();

                let stdout = match run_git(
                    state,
                    &repo_path,
                    &[
                        "log",
                        "--format=%H%x00%an%x00%ae%x00%at%x00%P%x00%s",
                        &format!("-n{limit_str}"),
                        &format!("--skip={skip_str}"),
                        git_ref,
                    ],
                )
                .await
                {
                    Ok(out) => {
                        debug!(bytes = out.len(), "git browse synthesis: git log output");
                        out
                    }
                    Err(_) => {
                        warn!(git_ref, ?repo_path, "git browse synthesis: git log failed");
                        return None;
                    }
                };

                let commits = parse_commit_log_output(&stdout);
                let content = match serde_json::to_string(&commits) {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(%e, "git browse synthesis: failed to serialize commits");
                        return None;
                    }
                };

                let tags = vec![
                    nostr::Tag::parse(&["d", d_tag]).ok()?,
                    nostr::Tag::parse(&["r", git_ref]).ok()?,
                    nostr::Tag::parse(&["page", &page.to_string()]).ok()?,
                ];

                match build_signed_event(state, KIND_GIT_COMMIT_LOG, &content, tags) {
                    Some(ev) => ev,
                    None => {
                        warn!("git browse synthesis: failed to build signed commit event");
                        return None;
                    }
                }
            }
            KIND_GIT_README => {
                let readme = match find_readme(state, &repo_path, git_ref).await {
                    Some(r) => r,
                    None => {
                        debug!(git_ref, ?repo_path, "git browse synthesis: no README found");
                        return None;
                    }
                };

                let tags = vec![
                    nostr::Tag::parse(&["d", d_tag]).ok()?,
                    nostr::Tag::parse(&["r", git_ref]).ok()?,
                    nostr::Tag::parse(&["filename", &readme.filename]).ok()?,
                ];

                match build_signed_event(state, KIND_GIT_README, &readme.content, tags) {
                    Some(ev) => ev,
                    None => {
                        warn!("git browse synthesis: failed to build signed readme event");
                        return None;
                    }
                }
            }
            _ => return None,
        };

        results.push(event);
    }

    debug!(count = results.len(), "git browse synthesis: completed");
    Some(results)
}

/// Build a synthetic event signed by the relay keypair.
fn build_signed_event(
    state: &AppState,
    kind: u32,
    content: &str,
    tags: Vec<nostr::Tag>,
) -> Option<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let event = match nostr::EventBuilder::new(nostr::Kind::Custom(kind as u16), content, tags)
        .custom_created_at(nostr::Timestamp::from(now))
        .sign_with_keys(&state.relay_keypair)
    {
        Ok(ev) => ev,
        Err(e) => {
            warn!(%e, kind, "git browse synthesis: sign_with_keys failed");
            return None;
        }
    };

    match serde_json::to_value(&event) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(%e, kind, "git browse synthesis: serde_json::to_value failed");
            None
        }
    }
}
