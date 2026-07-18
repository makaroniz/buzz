use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::Once,
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const EXPERIMENTS_FILE: &str = "desktop-experiments.json";

/// Process-local experiment state, lazily hydrated from the Rust-owned store on
/// first read so every reader (spawn boundary, frontend) sees persisted state
/// without an app-setup ordering requirement. A corrupt store fails closed:
/// the error is logged and the disabled default stays.
static ACP_TOP_LEVEL_SESSIONS: AtomicBool = AtomicBool::new(false);
static HYDRATE: Once = Once::new();

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct DesktopExperiments {
    acp_top_level_sessions: bool,
}

fn experiments_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data dir: {error}"))?
        .join(EXPERIMENTS_FILE))
}

fn load_experiments(path: &Path) -> Result<DesktopExperiments, String> {
    if !path.exists() {
        return Ok(DesktopExperiments::default());
    }
    let payload =
        fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn save_experiments(path: &Path, experiments: &DesktopExperiments) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(experiments)
        .map_err(|error| format!("failed to serialize experiments: {error}"))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("open {} for atomic write: {error}", path.display()))?;
    use std::io::Write;
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", path.display()))
}

/// Read the experiment, hydrating from the store on first access.
pub(crate) fn acp_top_level_sessions_enabled(app: &AppHandle) -> bool {
    HYDRATE.call_once(
        || match experiments_path(app).and_then(|path| load_experiments(&path)) {
            Ok(experiments) => {
                ACP_TOP_LEVEL_SESSIONS.store(experiments.acp_top_level_sessions, Ordering::Release);
            }
            Err(error) => {
                eprintln!("buzz-desktop: failed to hydrate desktop experiments: {error}");
            }
        },
    );
    ACP_TOP_LEVEL_SESSIONS.load(Ordering::Acquire)
}

#[tauri::command]
pub fn get_acp_top_level_sessions_experiment(app: AppHandle) -> bool {
    acp_top_level_sessions_enabled(&app)
}

/// Durably apply the experiment before exposing it to subsequently spawned agents.
#[tauri::command]
pub fn set_acp_top_level_sessions_experiment(enabled: bool, app: AppHandle) -> Result<(), String> {
    let path = experiments_path(&app)?;
    let mut experiments = load_experiments(&path)?;
    experiments.acp_top_level_sessions = enabled;
    save_experiments(&path, &experiments)?;
    // Persisted first: even if first-read hydration races this store, it
    // re-reads the same on-disk value.
    ACP_TOP_LEVEL_SESSIONS.store(enabled, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load_experiments, save_experiments, DesktopExperiments};

    #[test]
    fn missing_store_defaults_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_experiments(&dir.path().join("missing.json")).unwrap();
        assert!(!loaded.acp_top_level_sessions);
    }

    #[test]
    fn persisted_enabled_state_round_trips_for_fresh_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        save_experiments(
            &path,
            &DesktopExperiments {
                acp_top_level_sessions: true,
            },
        )
        .unwrap();
        let loaded = load_experiments(&path).unwrap();
        assert!(loaded.acp_top_level_sessions);
    }

    #[test]
    fn malformed_store_fails_closed_instead_of_enabling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop-experiments.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_experiments(&path).is_err());
    }
}
