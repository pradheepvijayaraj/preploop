use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;
use uuid::Uuid;

const PENDING_UPDATE_PACKAGE: &str = "pending-update.bin";
const PENDING_UPDATE_METADATA: &str = "pending-update.json";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdate {
    pub current_version: String,
    pub version: String,
    pub body: Option<String>,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    #[serde(rename = "Started")]
    Started {
        #[serde(rename = "contentLength")]
        content_length: Option<u64>,
    },
    #[serde(rename = "Progress")]
    Progress {
        #[serde(rename = "chunkLength")]
        chunk_length: usize,
    },
    #[serde(rename = "Finished")]
    Finished,
}

fn pending_paths(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve updater data directory: {error}"))?;
    Ok((
        directory.join(PENDING_UPDATE_PACKAGE),
        directory.join(PENDING_UPDATE_METADATA),
    ))
}

fn remove_pending_files(paths: &(PathBuf, PathBuf)) {
    for path in [&paths.0, &paths.1] {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "Could not remove pending updater file {}: {error}",
                    path.display()
                );
            }
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_pending_at(paths: &(PathBuf, PathBuf)) -> Result<Option<PendingUpdate>, String> {
    let package_exists = paths.0.is_file();
    let metadata_exists = paths.1.is_file();
    if !package_exists && !metadata_exists {
        return Ok(None);
    }
    if !package_exists || !metadata_exists {
        remove_pending_files(paths);
        return Ok(None);
    }

    let metadata_bytes = fs::read(&paths.1)
        .map_err(|error| format!("Could not read pending update metadata: {error}"))?;
    let pending = match serde_json::from_slice::<PendingUpdate>(&metadata_bytes) {
        Ok(pending) => pending,
        Err(error) => {
            log::warn!("Discarding invalid pending update metadata: {error}");
            remove_pending_files(paths);
            return Ok(None);
        }
    };
    let package = fs::read(&paths.0)
        .map_err(|error| format!("Could not read pending update package: {error}"))?;
    if pending.size != package.len() as u64 || pending.sha256 != sha256(&package) {
        log::warn!("Discarding incomplete or changed pending update package");
        remove_pending_files(paths);
        return Ok(None);
    }
    Ok(Some(pending))
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let temporary_path = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = File::create(&temporary_path)
            .map_err(|error| format!("Could not create updater temporary file: {error}"))?;
        file.write_all(contents)
            .map_err(|error| format!("Could not write updater temporary file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush updater temporary file: {error}"))?;
        if path.exists() {
            fs::remove_file(path)
                .map_err(|error| format!("Could not replace updater file: {error}"))?;
        }
        fs::rename(&temporary_path, path)
            .map_err(|error| format!("Could not commit updater file: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn persist_pending_update(
    app: &AppHandle,
    pending: &PendingUpdate,
    package: &[u8],
) -> Result<(), String> {
    let paths = pending_paths(app)?;
    if let Some(directory) = paths.0.parent() {
        fs::create_dir_all(directory)
            .map_err(|error| format!("Could not create updater data directory: {error}"))?;
    }

    // Commit the package before its metadata. If interrupted, the metadata
    // hash check discards the incomplete pair on the next launch.
    atomic_write(&paths.0, package)?;
    let metadata = serde_json::to_vec_pretty(pending)
        .map_err(|error| format!("Could not encode pending update metadata: {error}"))?;
    atomic_write(&paths.1, &metadata)
}

#[tauri::command]
pub fn get_pending_update(
    app: AppHandle,
    current_version: String,
) -> Result<Option<PendingUpdate>, String> {
    let paths = pending_paths(&app)?;
    let Some(pending) = read_pending_at(&paths)? else {
        return Ok(None);
    };
    if pending.current_version != current_version {
        log::info!(
            "Discarding pending update {} created for {}, current app is {}",
            pending.version,
            pending.current_version,
            current_version
        );
        remove_pending_files(&paths);
        return Ok(None);
    }
    Ok(Some(pending))
}

#[tauri::command]
pub async fn download_pending_update(
    app: AppHandle,
    expected_version: String,
    on_event: Channel<DownloadEvent>,
) -> Result<PendingUpdate, String> {
    let updater = app
        .updater_builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|error| format!("Could not initialise updater: {error}"))?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("Could not check for updater package: {error}"))?
        .ok_or_else(|| "No update is currently available".to_string())?;
    if update.version != expected_version {
        return Err(format!(
            "Update changed while downloading: expected {expected_version}, found {}",
            update.version
        ));
    }

    let mut first_chunk = true;
    let package = update
        .download(
            |chunk_length, content_length| {
                if first_chunk {
                    first_chunk = false;
                    let _ = on_event.send(DownloadEvent::Started { content_length });
                }
                let _ = on_event.send(DownloadEvent::Progress { chunk_length });
            },
            || {
                let _ = on_event.send(DownloadEvent::Finished);
            },
        )
        .await
        .map_err(|error| format!("Could not download updater package: {error}"))?;
    let pending = PendingUpdate {
        current_version: update.current_version,
        version: update.version,
        body: update.body,
        size: package.len() as u64,
        sha256: sha256(&package),
    };
    persist_pending_update(&app, &pending, &package)?;
    Ok(pending)
}

#[tauri::command]
pub async fn install_pending_update(
    app: AppHandle,
    expected_version: String,
) -> Result<(), String> {
    let paths = pending_paths(&app)?;
    let pending = read_pending_at(&paths)?
        .ok_or_else(|| "No verified pending update is available".to_string())?;
    let current_version = app.package_info().version.to_string();
    if pending.current_version != current_version {
        remove_pending_files(&paths);
        return Err("Pending update belongs to a different installed version".to_string());
    }
    if pending.version != expected_version {
        return Err(format!(
            "Pending update changed: expected {expected_version}, found {}",
            pending.version
        ));
    }

    let package = fs::read(&paths.0)
        .map_err(|error| format!("Could not read pending update package: {error}"))?;
    let updater = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| format!("Could not initialise updater: {error}"))?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("Could not verify the pending update release: {error}"))?
        .ok_or_else(|| "The pending update is no longer available".to_string())?;
    if update.version != pending.version {
        return Err(format!(
            "A newer update is available: pending {}, latest {}",
            pending.version, update.version
        ));
    }
    update
        .install(package)
        .map_err(|error| format!("Could not install pending update: {error}"))?;
    remove_pending_files(&paths);
    Ok(())
}
