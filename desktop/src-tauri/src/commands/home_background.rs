use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const SCRIPT_MAX_BYTES: u64 = 256 * 1024;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PickHomeBackgroundResult {
    Asset {
        path: String,
        file_name: Option<String>,
    },
    Script {
        content: String,
        file_name: Option<String>,
    },
}

fn dialog_filter(kind: &str) -> Result<(&'static str, &'static [&'static str]), String> {
    match kind {
        "image" => Ok((
            "Images",
            &["jpg", "jpeg", "png", "gif", "webp", "avif", "svg"],
        )),
        "video" => Ok(("Videos", &["mp4", "mov", "m4v", "webm", "mkv"])),
        "script" => Ok(("Animations", &["html", "htm", "js", "css", "txt"])),
        _ => Err("unsupported background file type".to_string()),
    }
}

#[tauri::command]
pub async fn pick_home_background_file(
    app: tauri::AppHandle,
    kind: String,
) -> Result<Option<PickHomeBackgroundResult>, String> {
    let (filter_name, filter_exts) = dialog_filter(&kind)?;
    let (tx, rx) = tokio::sync::oneshot::channel();

    app.dialog()
        .file()
        .add_filter(filter_name, filter_exts)
        .pick_file(move |path| {
            let _ = tx.send(path);
        });

    let selected = rx.await.map_err(|_| "dialog cancelled".to_string())?;
    let file_path = match selected {
        Some(path) => path,
        None => return Ok(None),
    };

    let path = file_path.as_path().ok_or("invalid path")?.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned);

    if kind == "script" {
        let metadata =
            std::fs::metadata(&path).map_err(|error| format!("failed to inspect file: {error}"))?;
        if metadata.len() > SCRIPT_MAX_BYTES {
            return Err("script file is too large; choose a file under 256 KB".to_string());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read file: {error}"))?;
        return Ok(Some(PickHomeBackgroundResult::Script {
            content,
            file_name,
        }));
    }

    let background_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?
        .join("home-backgrounds");
    std::fs::create_dir_all(&background_dir)
        .map_err(|error| format!("failed to prepare background directory: {error}"))?;

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| {
            if kind == "video" {
                "mp4".to_string()
            } else {
                "png".to_string()
            }
        });
    let persisted_path =
        background_dir.join(format!("home-background-{}.{}", Uuid::new_v4(), extension));
    std::fs::copy(&path, &persisted_path)
        .map_err(|error| format!("failed to copy background file: {error}"))?;

    Ok(Some(PickHomeBackgroundResult::Asset {
        path: persisted_path.to_string_lossy().into_owned(),
        file_name,
    }))
}
