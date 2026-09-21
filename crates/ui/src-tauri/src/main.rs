#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use desktop_snapshot_core::DesktopRoot;
use desktop_snapshot_platform_windows::{
    backup_contents, capture_layout, capture_screenshots, data_directory, desktop_locations,
    fences_installed, now_unix_seconds, open_in_explorer, register_backup_task, restore_layout,
    restore_missing_contents, run_as_admin_and_wait, scan_desktop,
};
use desktop_snapshot_storage::{SnapshotManifest, SnapshotStore, SnapshotSummary};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupResult {
    id: String,
    item_count: usize,
    backed_up_count: usize,
    failed_items: Vec<String>,
    screenshot_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreResult {
    id: String,
    restored_count: usize,
    skipped_count: usize,
    failed_count: usize,
    failed_items: Vec<String>,
    layout_restored: bool,
    elevation_requested: bool,
}

#[tauri::command]
fn get_fences_status() -> bool {
    fences_installed()
}
#[tauri::command]
fn list_snapshots() -> Result<Vec<SnapshotSummary>, String> {
    open_store()
        .map_err(|error| error.to_string())?
        .list_snapshots()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_snapshot() -> Result<BackupResult, String> {
    let locations = desktop_locations().map_err(|error| error.to_string())?;
    let mut items = scan_desktop().map_err(|error| error.to_string())?;
    let layout = capture_layout(&locations, &items).ok();
    let store = open_store().map_err(|error| error.to_string())?;
    let id = store
        .create_snapshot(now_unix_seconds(), &items, layout.clone())
        .map_err(|error| error.to_string())?;
    let report = backup_contents(&store.snapshot_dir(&id), &locations, &mut items);
    store
        .update_snapshot(&id, &items, layout)
        .map_err(|error| error.to_string())?;
    let screenshot_paths = capture_screenshots(&store.snapshot_dir(&id)).unwrap_or_default();
    store
        .update_screenshots(&id, screenshot_paths.clone())
        .map_err(|error| error.to_string())?;

    Ok(BackupResult {
        id,
        item_count: items.len(),
        backed_up_count: report.backed_up,
        failed_items: report.failed,
        screenshot_count: screenshot_paths.len(),
    })
}

#[tauri::command]
fn get_snapshot_images(snapshot_id: String, thumbnail: bool) -> Result<Vec<String>, String> {
    let store = open_store().map_err(|error| error.to_string())?;
    let manifest = store
        .load_manifest(&snapshot_id)
        .map_err(|error| error.to_string())?;
    if manifest.screenshots.is_empty() {
        return Err("该快照没有截图".to_owned());
    }

    manifest
        .screenshots
        .iter()
        .map(|stored_path| {
            let file_name = std::path::Path::new(stored_path)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "截图路径无效".to_owned())?;
            let target_name = if thumbnail {
                file_name.replacen("screen-", "thumb-", 1)
            } else {
                file_name.to_owned()
            };
            let image_path = store
                .snapshot_dir(&snapshot_id)
                .join("screenshots")
                .join(target_name);
            let bytes = fs::read(image_path).map_err(|error| error.to_string())?;
            Ok(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        })
        .collect()
}
fn restore_snapshot_data(snapshot_id: &str) -> Result<RestoreResult, String> {
    let locations = desktop_locations().map_err(|error| error.to_string())?;
    let store = open_store().map_err(|error| error.to_string())?;
    let manifest = store
        .load_manifest(snapshot_id)
        .map_err(|error| error.to_string())?;
    let report = restore_missing_contents(
        &store.snapshot_dir(snapshot_id),
        &locations,
        &manifest.items,
        None,
        false,
    );
    let layout_restored = if let Some(layout) = manifest.layout {
        restore_layout(&locations, &layout).map_err(|error| error.to_string())?;
        true
    } else {
        false
    };

    Ok(RestoreResult {
        id: snapshot_id.to_owned(),
        restored_count: report.restored.len(),
        skipped_count: report.skipped.len(),
        failed_count: report.failed.len(),
        failed_items: report.failed_details,
        layout_restored,
        elevation_requested: false,
    })
}

fn needs_public_desktop_elevation(
    manifest: &SnapshotManifest,
    locations: &[desktop_snapshot_platform_windows::DesktopLocation],
) -> bool {
    let Some(public_desktop) = locations
        .iter()
        .find(|location| location.root == DesktopRoot::Public)
    else {
        return false;
    };
    manifest.items.iter().any(|item| {
        item.root == DesktopRoot::Public && !public_desktop.path.join(&item.relative_path).exists()
    })
}

#[tauri::command]
fn restore_snapshot(snapshot_id: String) -> Result<RestoreResult, String> {
    let locations = desktop_locations().map_err(|error| error.to_string())?;
    let store = open_store().map_err(|error| error.to_string())?;
    let manifest = store
        .load_manifest(&snapshot_id)
        .map_err(|error| error.to_string())?;
    if needs_public_desktop_elevation(&manifest, &locations) {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        run_as_admin_and_wait(
            &executable,
            &["--elevated-restore".to_owned(), snapshot_id.clone()],
        )
        .map_err(|error| error.to_string())?;
        return Ok(RestoreResult {
            id: snapshot_id,
            restored_count: 0,
            skipped_count: 0,
            failed_count: 0,
            failed_items: Vec::new(),
            layout_restored: false,
            elevation_requested: true,
        });
    }
    restore_snapshot_data(&snapshot_id)
}

#[tauri::command]
fn delete_snapshot(snapshot_id: String) -> Result<SnapshotSummary, String> {
    open_store()
        .map_err(|error| error.to_string())?
        .delete_snapshot(&snapshot_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn rename_snapshot(snapshot_id: String, title: String) -> Result<SnapshotSummary, String> {
    open_store()
        .map_err(|error| error.to_string())?
        .rename_snapshot(&snapshot_id, title)
        .map_err(|error| error.to_string())
}
#[tauri::command]
fn open_backup_directory() -> Result<String, String> {
    let directory = data_directory().map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    open_in_explorer(&directory).map_err(|error| error.to_string())?;
    Ok(directory.display().to_string())
}

#[tauri::command]
fn register_auto_backup(interval_minutes: u32) -> Result<(), String> {
    let cli_path = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("desktop-snapshot.exe");
    register_backup_task(&cli_path, interval_minutes).map_err(|error| error.to_string())
}

fn open_store() -> Result<SnapshotStore, Box<dyn std::error::Error>> {
    let directory: PathBuf = data_directory()?;
    fs::create_dir_all(&directory)?;
    Ok(SnapshotStore::open(directory)?)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--elevated-restore")
    {
        if let Some(snapshot_id) = args.get(index + 1) {
            if let Err(error) = restore_snapshot_data(snapshot_id) {
                eprintln!("管理员恢复失败：{error}");
                std::process::exit(1);
            }
            return;
        }
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_fences_status,
            list_snapshots,
            create_snapshot,
            get_snapshot_images,
            restore_snapshot,
            delete_snapshot,
            rename_snapshot,
            open_backup_directory,
            register_auto_backup,
        ])
        .run(tauri::generate_context!())
        .expect("启动 DesktopSnapshot Tauri 应用失败");
}
