//! PATH augmentation for launched managed-agent child processes.

use std::path::PathBuf;

/// Assemble the augmented `PATH` for a launched managed-agent child process.
///
/// Concatenates, in priority order:
///   1. `bundled_acp_bin` — bundled ACP bridge tools dir, so pinned bridges
///      shipped with the app win over user-installed copies
///   2. `<home>/.local/bin` — bundled CLI symlink
///   3. `nvm_bin` — nvm's default Node.js bin dir (if the user uses nvm)
///   4. exe parent dir — DMG sidecars under `Contents/MacOS/`
///   5. user's login-shell `PATH` — runtimes like node/python from other managers
///
/// `shell_path` is the raw colon-delimited string from a login shell, so it is
/// split into individual entries before joining. Pushing it as a single segment
/// would make `join_paths` reject it (a segment containing the separator is an
/// error), collapsing the entire augmented `PATH` to `None` — the bug this
/// guards against, which left managed agents unable to find `buzz`. Returns
/// `None` only when no entries exist.
pub(in crate::managed_agents) fn build_augmented_path(
    bundled_acp_bin: Option<PathBuf>,
    home: Option<PathBuf>,
    exe_parent: Option<PathBuf>,
    shell_path: Option<String>,
    nvm_bin: Option<PathBuf>,
) -> Option<String> {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(bundled) = bundled_acp_bin {
        parts.push(bundled);
    }
    if let Some(home) = home {
        parts.push(home.join(".local").join("bin"));
    }
    if let Some(nvm_bin) = nvm_bin {
        parts.push(nvm_bin);
    }
    if let Some(parent) = exe_parent {
        parts.push(parent);
    }
    if let Some(shell_path) = shell_path {
        parts.extend(std::env::split_paths(&shell_path));
    }
    join_paths_best_effort(parts)
}

/// Join PATH entries, degrading to a best-effort join when a single entry
/// embeds the platform separator (legal in macOS paths): `join_paths` rejects
/// the whole list for one such entry, which would collapse the entire
/// augmented `PATH` to `None` and hand child processes a bare GUI PATH. Drop
/// the un-joinable entries (logging each) instead of erasing every search
/// path. Returns `None` only when no joinable entries exist.
fn join_paths_best_effort(mut paths: Vec<PathBuf>) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    // join_paths uses the platform separator (':' on Unix, ';' on Windows).
    if let Ok(joined) = std::env::join_paths(&paths) {
        return Some(joined.to_string_lossy().into_owned());
    }
    paths.retain(|path| {
        let joinable = std::env::join_paths(std::iter::once(path)).is_ok();
        if !joinable {
            eprintln!(
                "buzz-desktop: dropping un-joinable PATH entry: {}",
                path.display()
            );
        }
        joinable
    });
    if paths.is_empty() {
        return None;
    }
    std::env::join_paths(paths)
        .ok()
        .map(|s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::build_augmented_path;
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn splits_colon_delimited_shell_path() {
        // Regression: the shell PATH arrives as one colon-delimited string. It
        // must be split into segments before join_paths, or join_paths rejects
        // it and the whole augmented PATH collapses to None (managed agents then
        // lose `buzz`).
        let result = build_augmented_path(
            None,
            Some(PathBuf::from("/home/agent")),
            Some(PathBuf::from("/Applications/Buzz.app/Contents/MacOS")),
            Some("/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin".to_string()),
            None,
        );
        assert_eq!(
            result.as_deref(),
            Some(
                "/home/agent/.local/bin:/Applications/Buzz.app/Contents/MacOS:\
/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin"
            ),
        );
    }

    #[test]
    fn none_when_no_inputs() {
        assert_eq!(build_augmented_path(None, None, None, None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_only() {
        let result =
            build_augmented_path(None, None, None, Some("/usr/bin:/bin".to_string()), None);
        assert_eq!(result.as_deref(), Some("/usr/bin:/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn bundled_acp_bin_is_highest_priority_segment() {
        let result = build_augmented_path(
            Some(PathBuf::from(
                "/Applications/Buzz.app/Contents/Resources/resources/acp/bin",
            )),
            Some(PathBuf::from("/home/user")),
            Some(PathBuf::from("/Applications/Buzz.app/Contents/MacOS")),
            Some("/usr/bin:/bin".to_string()),
            Some(PathBuf::from("/home/user/.nvm/versions/node/v20.0.0/bin")),
        );
        assert_eq!(
            result.as_deref(),
            Some(
                "/Applications/Buzz.app/Contents/Resources/resources/acp/bin:\
/home/user/.local/bin:\
/home/user/.nvm/versions/node/v20.0.0/bin:\
/Applications/Buzz.app/Contents/MacOS:\
/usr/bin:/bin"
            ),
        );
    }

    #[cfg(unix)]
    #[test]
    fn nvm_bin_inserted_after_local_bin_before_exe_parent() {
        let result = build_augmented_path(
            None,
            Some(PathBuf::from("/home/user")),
            Some(PathBuf::from("/Applications/Buzz.app/Contents/MacOS")),
            Some("/usr/bin:/bin".to_string()),
            Some(PathBuf::from("/home/user/.nvm/versions/node/v20.0.0/bin")),
        );
        assert_eq!(
            result.as_deref(),
            Some(
                "/home/user/.local/bin:\
/home/user/.nvm/versions/node/v20.0.0/bin:\
/Applications/Buzz.app/Contents/MacOS:\
/usr/bin:/bin"
            ),
        );
    }

    #[cfg(unix)]
    #[test]
    fn nvm_bin_none_does_not_add_segment() {
        let result = build_augmented_path(
            None,
            Some(PathBuf::from("/home/user")),
            Some(PathBuf::from("/usr/local/bin")),
            None,
            None,
        );
        assert_eq!(
            result.as_deref(),
            Some("/home/user/.local/bin:/usr/local/bin"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn unjoinable_entry_is_dropped_instead_of_emptying_path() {
        // A dir embedding the separator (legal in macOS paths) can't be joined
        // into PATH; it must be dropped, not collapse the whole augmented PATH
        // to None.
        let result = build_augmented_path(
            Some(PathBuf::from("/weird:dir/bin")),
            None,
            None,
            Some("/shell/bin:/user/bin".to_string()),
            None,
        );
        let path = result.expect("PATH should survive an un-joinable entry");
        let paths: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(
            paths,
            vec![PathBuf::from("/shell/bin"), PathBuf::from("/user/bin")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn none_when_all_entries_unjoinable() {
        let result = build_augmented_path(
            Some(PathBuf::from("/weird:dir/bin")),
            None,
            None,
            None,
            None,
        );
        assert_eq!(result, None);
    }
}
