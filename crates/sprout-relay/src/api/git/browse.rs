//! Git browse API — unauthenticated read-only endpoints for the web portal.
//!
//! **Auth note:** These REST endpoints are intentionally unauthenticated (public
//! read-only) for the web portal, unlike the git transport routes which require
//! NIP-98 auth. The primary consumption path for the SPA is the event synthesis
//! pipeline (bridge.rs → synthesis.rs), which IS authenticated via NIP-98. These
//! REST endpoints serve as a simpler fallback (curl-friendly, cacheable). Auth
//! can be added here later if the product decision changes.
//!
//! Four endpoints:
//! - `GET /api/repos/{owner}/{repo}/tree/{ref}/{*path}` — list directory entries
//! - `GET /api/repos/{owner}/{repo}/blob/{ref}/{*path}` — read file content
//! - `GET /api/repos/{owner}/{repo}/commits/{ref}` — commit log
//! - `GET /api/repos/{owner}/{repo}/readme/{ref}` — README file content

use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::error;

use crate::api::api_error;
use crate::state::AppState;

use super::transport::{harden_git_env, validate_repo_path};

/// Max file size for blob endpoint (1 MB).
const MAX_BLOB_SIZE: u64 = 1024 * 1024;

// ── Shared Helpers ──────────────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
pub(crate) fn validate_ref(git_ref: &str) -> Result<(), Response> {
    if git_ref.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "ref must not be empty").into_response());
    }
    if git_ref.contains('\0')
        || git_ref.contains("..")
        || git_ref.contains('\\')
        || git_ref.as_bytes().iter().any(|&b| b < 0x20)
    {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "ref contains invalid characters").into_response(),
        );
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
pub(crate) fn validate_tree_path(path: &str) -> Result<(), Response> {
    // Empty path is fine (root tree)
    if path.is_empty() {
        return Ok(());
    }
    if path.contains('\0') || path.as_bytes().iter().any(|&b| b < 0x20) {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "path contains invalid characters").into_response(),
        );
    }
    if path.starts_with('/') {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "path must not be absolute").into_response(),
        );
    }
    // Check each path component for ".."
    for component in path.split('/') {
        if component == ".." {
            return Err(
                api_error(StatusCode::BAD_REQUEST, "path traversal not allowed").into_response(),
            );
        }
    }
    Ok(())
}

/// Run a git command against a bare repo, acquiring the semaphore first.
/// Returns stdout bytes on success, or an error response.
pub(crate) async fn run_git(
    state: &AppState,
    repo_path: &Path,
    args: &[&str],
) -> Result<Vec<u8>, Response> {
    let _permit = state.git_semaphore.acquire().await.map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "git service temporarily unavailable",
        )
        .into_response()
    })?;

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_path);
    cmd.args(args);
    harden_git_env(&mut cmd);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await.map_err(|e| {
        error!("failed to spawn git: {e}");
        api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Distinguish "not found" errors from real failures
        if stderr.contains("not a valid object name")
            || stderr.contains("does not exist")
            || stderr.contains("Not a valid object name")
            || stderr.contains("bad revision")
            || stderr.contains("fatal: not a git repository")
            || stderr.contains("not a tree object")
        {
            return Err(api_error(StatusCode::NOT_FOUND, "not found").into_response());
        }
        error!("git command failed: {stderr}");
        return Err(
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response(),
        );
    }

    Ok(output.stdout)
}

/// Verify a ref exists using `git rev-parse --verify`.
pub(crate) async fn verify_ref(
    state: &AppState,
    repo_path: &Path,
    git_ref: &str,
) -> Result<(), Response> {
    run_git(state, repo_path, &["rev-parse", "--verify", git_ref]).await?;
    Ok(())
}

#[allow(clippy::result_large_err)]
pub(crate) fn resolve_repo(
    owner: &str,
    repo: &str,
    state: &AppState,
) -> Result<std::path::PathBuf, Response> {
    let validated = validate_repo_path(owner, repo, &state.config.git_repo_path)?;
    let path = validated.repo_path;
    if !path.exists() {
        return Err(api_error(StatusCode::NOT_FOUND, "repository not found").into_response());
    }
    Ok(path)
}

// ── Reusable parsing functions (used by both REST endpoints and event synthesis) ──

/// Parse `git ls-tree --long` output into tree entries.
pub(crate) fn parse_tree_output(stdout: &[u8]) -> Vec<TreeEntry> {
    let text = String::from_utf8_lossy(stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((meta, name)) = line.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = meta.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let mode = parts[0];
        let entry_type = parts[1];
        let sha = parts[2];
        let size_str = parts[3];
        let size = if entry_type == "blob" {
            size_str.parse::<u64>().ok()
        } else {
            None
        };
        entries.push(TreeEntry {
            name: name.to_string(),
            entry_type: entry_type.to_string(),
            mode: mode.to_string(),
            size,
            sha: sha.to_string(),
        });
    }
    entries
}

/// Parse `git log --format=...` output into commit entries.
pub(crate) fn parse_commit_log_output(stdout: &[u8]) -> Vec<CommitEntry> {
    let text = String::from_utf8_lossy(stdout);
    let mut commits = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(6, '\0').collect();
        if fields.len() < 6 {
            continue;
        }
        let parents = if fields[4].is_empty() {
            vec![]
        } else {
            fields[4].split(' ').map(|s| s.to_string()).collect()
        };
        commits.push(CommitEntry {
            sha: fields[0].to_string(),
            author: fields[1].to_string(),
            email: fields[2].to_string(),
            timestamp: fields[3].parse().unwrap_or(0),
            parents,
            message: fields[5].to_string(),
        });
    }
    commits
}

/// Blob metadata returned by `resolve_blob_metadata`.
pub(crate) struct BlobMetadata {
    pub size: u64,
    pub is_binary: bool,
}

/// Resolve blob metadata (size, binary detection) for a file path at a given ref.
pub(crate) async fn resolve_blob_metadata(
    state: &AppState,
    repo_path: &std::path::Path,
    git_ref: &str,
    file_path: &str,
) -> Result<BlobMetadata, Response> {
    let ls_output = run_git(state, repo_path, &["ls-tree", git_ref, file_path]).await?;
    let ls_text = String::from_utf8_lossy(&ls_output);
    let ls_line = ls_text
        .lines()
        .next()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "file not found").into_response())?;
    let Some((meta, _)) = ls_line.split_once('\t') else {
        return Err(api_error(StatusCode::NOT_FOUND, "file not found").into_response());
    };
    let parts: Vec<&str> = meta.split_whitespace().collect();
    if parts.len() < 3 || parts[1] != "blob" {
        return Err(api_error(StatusCode::BAD_REQUEST, "path is not a file").into_response());
    }
    let sha = parts[2];

    let size_output = run_git(state, repo_path, &["cat-file", "-s", sha]).await?;
    let size: u64 = String::from_utf8_lossy(&size_output)
        .trim()
        .parse()
        .unwrap_or(0);

    // Binary detection: read first 512 bytes
    let is_binary = if size > 0 && size <= MAX_BLOB_SIZE {
        let content = run_git(state, repo_path, &["cat-file", "-p", sha]).await?;
        content.iter().take(512).any(|&b| b == 0)
    } else {
        // For oversized files, assume binary to be safe
        size > MAX_BLOB_SIZE
    };

    Ok(BlobMetadata { size, is_binary })
}

/// Try to find and read a README file at a given ref.
///
/// Enforces the same `MAX_BLOB_SIZE` (1 MB) limit as the blob endpoint
/// to prevent memory exhaustion on oversized README files.
pub(crate) async fn find_readme(
    state: &AppState,
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Option<ReadmeResponse> {
    let candidates = ["README.md", "README", "README.rst", "README.txt"];
    for filename in candidates {
        let target = format!("{git_ref}:{filename}");
        // Check size before reading to avoid loading oversized files into memory.
        let Ok(size_output) = run_git(state, repo_path, &["cat-file", "-s", &target]).await else {
            continue;
        };
        let size: u64 = String::from_utf8_lossy(&size_output)
            .trim()
            .parse()
            .unwrap_or(0);
        if size > MAX_BLOB_SIZE {
            continue;
        }
        if let Ok(content) = run_git(state, repo_path, &["cat-file", "-p", &target]).await {
            let text = String::from_utf8_lossy(&content).into_owned();
            return Some(ReadmeResponse {
                filename: filename.to_string(),
                content: text,
            });
        }
    }
    None
}

pub(crate) const COMMITS_PER_PAGE: u32 = 20;

// ── Tree Endpoint ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct TreeEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub mode: String,
    pub size: Option<u64>,
    pub sha: String,
}

/// `GET /api/repos/{owner}/{repo}/tree/{ref}/{*path}`
async fn tree_handler(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, git_ref, path)): AxumPath<(String, String, String, String)>,
) -> Result<Json<Vec<TreeEntry>>, Response> {
    validate_ref(&git_ref)?;
    validate_tree_path(&path)?;
    let repo_path = resolve_repo(&owner, &repo, &state)?;
    verify_ref(&state, &repo_path, &git_ref).await?;

    let target = if path.is_empty() {
        git_ref.clone()
    } else {
        let clean_path = path.strip_suffix('/').unwrap_or(&path);
        format!("{git_ref}:{clean_path}")
    };

    let stdout = run_git(&state, &repo_path, &["ls-tree", "--long", &target]).await?;
    Ok(Json(parse_tree_output(&stdout)))
}

/// `GET /api/repos/{owner}/{repo}/tree/{ref}` — root tree (no path).
async fn tree_root_handler(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, git_ref)): AxumPath<(String, String, String)>,
) -> Result<Json<Vec<TreeEntry>>, Response> {
    validate_ref(&git_ref)?;
    let repo_path = resolve_repo(&owner, &repo, &state)?;
    verify_ref(&state, &repo_path, &git_ref).await?;

    let stdout = run_git(&state, &repo_path, &["ls-tree", "--long", &git_ref]).await?;
    Ok(Json(parse_tree_output(&stdout)))
}

// ── Blob Endpoint ───────────────────────────────────────────────────────────

/// `GET /api/repos/{owner}/{repo}/blob/{ref}/{*path}`
async fn blob_handler(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, git_ref, path)): AxumPath<(String, String, String, String)>,
) -> Result<Response, Response> {
    validate_ref(&git_ref)?;
    validate_tree_path(&path)?;

    if path.is_empty() {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "path is required for blob").into_response(),
        );
    }

    let repo_path = resolve_repo(&owner, &repo, &state)?;
    verify_ref(&state, &repo_path, &git_ref).await?;

    let clean_path = path.strip_suffix('/').unwrap_or(&path);

    // Resolve blob SHA via ls-tree
    let ls_output = run_git(&state, &repo_path, &["ls-tree", &git_ref, clean_path]).await?;

    let ls_text = String::from_utf8_lossy(&ls_output);
    let ls_line = ls_text
        .lines()
        .next()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "file not found").into_response())?;

    let Some((meta, _name)) = ls_line.split_once('\t') else {
        return Err(api_error(StatusCode::NOT_FOUND, "file not found").into_response());
    };
    let parts: Vec<&str> = meta.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(api_error(StatusCode::NOT_FOUND, "file not found").into_response());
    }
    let obj_type = parts[1];
    let sha = parts[2];

    if obj_type != "blob" {
        return Err(api_error(StatusCode::BAD_REQUEST, "path is not a file").into_response());
    }

    // Check size before reading content
    let size_output = run_git(&state, &repo_path, &["cat-file", "-s", sha]).await?;
    let size_str = String::from_utf8_lossy(&size_output).trim().to_string();
    let size: u64 = size_str.parse().unwrap_or(0);

    if size > MAX_BLOB_SIZE {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            [(
                header::HeaderName::from_static("x-file-size"),
                size.to_string(),
            )],
            Json(serde_json::json!({ "error": "file too large", "size": size })),
        )
            .into_response());
    }

    // Read content
    let content = run_git(&state, &repo_path, &["cat-file", "-p", sha]).await?;

    // Binary detection: check first 512 bytes for NUL byte
    let is_binary = content.iter().take(512).any(|&b| b == 0);

    if is_binary {
        Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::HeaderName::from_static("x-file-size"),
                    size.to_string(),
                ),
            ],
            content,
        )
            .into_response())
    } else {
        Ok((
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    "text/plain; charset=utf-8".to_string(),
                ),
                (
                    header::HeaderName::from_static("x-file-size"),
                    size.to_string(),
                ),
            ],
            content,
        )
            .into_response())
    }
}

// ── Commits Endpoint ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CommitEntry {
    pub sha: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub parents: Vec<String>,
    pub message: String,
}

#[derive(Deserialize)]
struct CommitsQuery {
    page: Option<u32>,
}

/// `GET /api/repos/{owner}/{repo}/commits/{ref}`
async fn commits_handler(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, git_ref)): AxumPath<(String, String, String)>,
    Query(query): Query<CommitsQuery>,
) -> Result<Json<Vec<CommitEntry>>, Response> {
    validate_ref(&git_ref)?;
    let repo_path = resolve_repo(&owner, &repo, &state)?;
    verify_ref(&state, &repo_path, &git_ref).await?;

    let page = query.page.unwrap_or(0);
    let skip = page * COMMITS_PER_PAGE;
    let skip_str = skip.to_string();
    let limit_str = COMMITS_PER_PAGE.to_string();

    let stdout = run_git(
        &state,
        &repo_path,
        &[
            "log",
            "--format=%H%x00%an%x00%ae%x00%at%x00%P%x00%s",
            &format!("-n{limit_str}"),
            &format!("--skip={skip_str}"),
            &git_ref,
        ],
    )
    .await?;

    Ok(Json(parse_commit_log_output(&stdout)))
}

// ── Readme Endpoint ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ReadmeResponse {
    pub filename: String,
    pub content: String,
}

/// `GET /api/repos/{owner}/{repo}/readme/{ref}`
async fn readme_handler(
    State(state): State<Arc<AppState>>,
    AxumPath((owner, repo, git_ref)): AxumPath<(String, String, String)>,
) -> Result<Json<ReadmeResponse>, Response> {
    validate_ref(&git_ref)?;
    let repo_path = resolve_repo(&owner, &repo, &state)?;
    verify_ref(&state, &repo_path, &git_ref).await?;

    match find_readme(&state, &repo_path, &git_ref).await {
        Some(readme) => Ok(Json(readme)),
        None => Err(api_error(StatusCode::NOT_FOUND, "no readme found").into_response()),
    }
}

// ── Router ──────────────────────────────────────────────────────────────────

/// Build the git browse router with all 4 read-only endpoints.
pub fn browse_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/repos/{owner}/{repo}/tree/{ref}",
            get(tree_root_handler),
        )
        .route(
            "/api/repos/{owner}/{repo}/tree/{ref}/{*path}",
            get(tree_handler),
        )
        .route(
            "/api/repos/{owner}/{repo}/blob/{ref}/{*path}",
            get(blob_handler),
        )
        .route(
            "/api/repos/{owner}/{repo}/commits/{ref}",
            get(commits_handler),
        )
        .route(
            "/api/repos/{owner}/{repo}/readme/{ref}",
            get(readme_handler),
        )
        .with_state(state)
}
