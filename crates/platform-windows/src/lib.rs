mod layout;

pub use layout::{capture_layout, restore_layout};

use desktop_snapshot_core::{DesktopItem, DesktopItemKind, DesktopRoot};
use lnk::ShellLink;
use lnk::encoding::WINDOWS_1252;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::slice;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("无法读取桌面目录 {path}: {source}")]
    ReadDesktop { path: PathBuf, source: io::Error },
    #[error("无法读取桌面项目 {path}: {source}")]
    ReadItem { path: PathBuf, source: io::Error },
    #[error("无法复制内容 {source_path} -> {target_path}: {error}")]
    CopyContent {
        source_path: PathBuf,
        target_path: PathBuf,
        error: io::Error,
    },
    #[error("任务计划操作失败: {0}")]
    TaskScheduler(String),
    #[error("无法打开目录: {0}")]
    OpenPath(String),
    #[error("截图失败: {0}")]
    Screenshot(String),
    #[error("管理员权限操作失败: {0}")]
    Elevation(String),
    #[error("Windows API 错误: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("当前平台不是 Windows")]
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
pub struct DesktopLocation {
    pub root: DesktopRoot,
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct ContentBackupReport {
    pub backed_up: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub failed_details: Vec<String>,
}

pub fn desktop_locations() -> Result<Vec<DesktopLocation>, PlatformError> {
    #[cfg(windows)]
    {
        Ok(vec![
            DesktopLocation {
                root: DesktopRoot::User,
                path: known_folder_path(&windows::Win32::UI::Shell::FOLDERID_Desktop)?,
            },
            DesktopLocation {
                root: DesktopRoot::Public,
                path: known_folder_path(&windows::Win32::UI::Shell::FOLDERID_PublicDesktop)?,
            },
        ])
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::UnsupportedPlatform)
    }
}

pub fn scan_desktop() -> Result<Vec<DesktopItem>, PlatformError> {
    let mut items = Vec::new();
    for location in desktop_locations()? {
        if !location.path.exists() {
            continue;
        }
        let entries =
            fs::read_dir(&location.path).map_err(|source| PlatformError::ReadDesktop {
                path: location.path.clone(),
                source,
            })?;
        for entry in entries {
            let entry = entry.map_err(|source| PlatformError::ReadDesktop {
                path: location.path.clone(),
                source,
            })?;
            let path = entry.path();
            let metadata = fs::metadata(&path).map_err(|source| PlatformError::ReadItem {
                path: path.clone(),
                source,
            })?;
            let relative_path = path
                .strip_prefix(&location.path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let kind = classify(&path, metadata.is_dir());
            let (target_path, target_exists) = if kind == DesktopItemKind::Shortcut {
                shortcut_target(&path)
            } else {
                (None, None)
            };
            let modified_unix_seconds = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or_default();
            items.push(DesktopItem {
                root: location.root,
                relative_path,
                kind,
                size: if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                modified_unix_seconds,
                target_path,
                target_exists,
                content_backed_up: false,
            });
        }
    }
    items.sort_by_key(|item| (item.root, item.relative_path.clone()));
    Ok(items)
}

struct DesktopWindowGuard {
    windows: Vec<windows::Win32::Foundation::HWND>,
}

impl Drop for DesktopWindowGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            for window in &self.windows {
                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                    *window,
                    windows::Win32::UI::WindowsAndMessaging::SW_RESTORE,
                );
            }
        }
    }
}

fn minimize_desktop_windows() -> Result<DesktopWindowGuard, PlatformError> {
    #[cfg(windows)]
    {
        let mut windows = Vec::new();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::EnumWindows(
                Some(collect_window),
                windows::Win32::Foundation::LPARAM(
                    &mut windows as *mut Vec<windows::Win32::Foundation::HWND> as isize,
                ),
            )?;
        }
        Ok(DesktopWindowGuard { windows })
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
unsafe extern "system" fn collect_window(
    window: windows::Win32::Foundation::HWND,
    data: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    let windows = unsafe { &mut *(data.0 as *mut Vec<windows::Win32::Foundation::HWND>) };
    if !unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window) }.as_bool()
        || unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(window) }.as_bool()
    {
        return windows::core::BOOL(1);
    }
    let mut process_id = 0;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            window,
            Some(&mut process_id),
        );
    }
    if process_id == std::process::id() {
        return windows::core::BOOL(1);
    }
    let mut class_name = [0u16; 128];
    let length =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(window, &mut class_name) }
            as usize;
    let class_name = String::from_utf16_lossy(&class_name[..length]);
    if matches!(
        class_name.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        return windows::core::BOOL(1);
    }
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
            window,
            windows::Win32::UI::WindowsAndMessaging::SW_MINIMIZE,
        );
    }
    windows.push(window);
    windows::core::BOOL(1)
}
pub fn capture_screenshots(snapshot_dir: &Path) -> Result<Vec<String>, PlatformError> {
    let _window_guard = minimize_desktop_windows()?;
    thread::sleep(Duration::from_millis(250));
    #[cfg(windows)]
    {
        let screenshot_dir = snapshot_dir.join("screenshots");
        fs::create_dir_all(&screenshot_dir)
            .map_err(|error| PlatformError::Screenshot(error.to_string()))?;
        let mut screens = screenshots::Screen::all()
            .map_err(|error| PlatformError::Screenshot(error.to_string()))?;
        screens.sort_by_key(|screen| !screen.display_info.is_primary);
        let mut paths = Vec::new();
        for (index, screen) in screens.iter().enumerate() {
            let image = screen
                .capture()
                .map_err(|error| PlatformError::Screenshot(error.to_string()))?;
            let full_name = format!("screen-{index}.png");
            let thumb_name = format!("thumb-{index}.png");
            image
                .save(screenshot_dir.join(&full_name))
                .map_err(|error| PlatformError::Screenshot(error.to_string()))?;
            let thumbnail = screenshots::image::imageops::thumbnail(&image, 480, 270);
            thumbnail
                .save(screenshot_dir.join(&thumb_name))
                .map_err(|error| PlatformError::Screenshot(error.to_string()))?;
            paths.push(format!("screenshots/{full_name}"));
        }
        Ok(paths)
    }
    #[cfg(not(windows))]
    {
        let _ = snapshot_dir;
        Err(PlatformError::UnsupportedPlatform)
    }
}
pub fn backup_contents(
    snapshot_dir: &Path,
    locations: &[DesktopLocation],
    items: &mut [DesktopItem],
) -> ContentBackupReport {
    let mut report = ContentBackupReport::default();
    for item in items {
        let Some(location) = locations.iter().find(|location| location.root == item.root) else {
            report.failed.push(item.relative_path.clone());
            continue;
        };
        let source = location.path.join(&item.relative_path);
        let target = content_path(snapshot_dir, item);
        let result = if source.is_dir() {
            copy_directory(&source, &target)
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).and_then(|_| fs::copy(&source, &target).map(|_| ()))
            } else {
                fs::copy(&source, &target).map(|_| ())
            }
        };
        match result {
            Ok(()) => {
                item.content_backed_up = true;
                report.backed_up += 1;
            }
            Err(_) => report.failed.push(item.relative_path.clone()),
        }
    }
    report
}

fn record_restore_failure(report: &mut RestoreReport, item: &DesktopItem, detail: String) {
    report.failed.push(item.relative_path.clone());
    report.failed_details.push(detail);
}
pub fn restore_missing_contents(
    snapshot_dir: &Path,
    locations: &[DesktopLocation],
    items: &[DesktopItem],
    only_item: Option<&str>,
    overwrite: bool,
) -> RestoreReport {
    let mut report = RestoreReport::default();
    for item in items {
        if only_item.is_some_and(|value| value != item.relative_path) {
            continue;
        }
        let Some(location) = locations.iter().find(|location| location.root == item.root) else {
            record_restore_failure(
                &mut report,
                item,
                format!("{}（恢复失败）", item.relative_path),
            );
            continue;
        };
        let source = content_path(snapshot_dir, item);
        let target = location.path.join(&item.relative_path);
        if !source.exists() {
            record_restore_failure(
                &mut report,
                item,
                format!("{}（恢复失败）", item.relative_path),
            );
            continue;
        }
        if target.exists() && !overwrite {
            report.skipped.push(item.relative_path.clone());
            continue;
        }
        let result = if source.is_dir() {
            if overwrite && target.exists() {
                fs::remove_dir_all(&target).and_then(|_| copy_directory(&source, &target))
            } else if target.exists() {
                merge_directory(&source, &target)
            } else {
                copy_directory(&source, &target)
            }
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).and_then(|_| fs::copy(&source, &target).map(|_| ()))
            } else {
                fs::copy(&source, &target).map(|_| ())
            }
        };
        match result {
            Ok(()) => report.restored.push(item.relative_path.clone()),
            Err(error) => {
                let reason = if item.root == DesktopRoot::Public
                    && error.kind() == io::ErrorKind::PermissionDenied
                {
                    format!("{}（公共桌面需要管理员权限）", item.relative_path)
                } else {
                    format!("{}（{}）", item.relative_path, error)
                };
                record_restore_failure(&mut report, item, reason);
            }
        }
    }
    report
}

pub fn register_backup_task(cli_path: &Path, interval_minutes: u32) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        let task_name = "DesktopSnapshot\\自动备份";
        let task_command = format!(r#""{}" backup --quiet"#, cli_path.display());
        let status = Command::new("schtasks.exe")
            .args([
                "/Create",
                "/TN",
                task_name,
                "/TR",
                &task_command,
                "/SC",
                "MINUTE",
                "/MO",
                &interval_minutes.max(1).to_string(),
                "/F",
                "/RL",
                "LIMITED",
            ])
            .status()
            .map_err(|error| PlatformError::TaskScheduler(error.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::TaskScheduler(format!(
                "schtasks.exe 返回码 {:?}",
                status.code()
            )))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (cli_path, interval_minutes);
        Err(PlatformError::UnsupportedPlatform)
    }
}

pub fn set_window_icon_by_title(title: &str) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let resource_id = windows::core::PCWSTR(1 as *const u16);
        let window = unsafe {
            windows::Win32::UI::WindowsAndMessaging::FindWindowW(
                None,
                windows::core::PCWSTR(title_wide.as_ptr()),
            )?
        };
        let module = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };
        let icon = unsafe {
            windows::Win32::UI::WindowsAndMessaging::LoadIconW(
                Some(windows::Win32::Foundation::HINSTANCE(module.0)),
                resource_id,
            )?
        };
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                window,
                windows::Win32::UI::WindowsAndMessaging::WM_SETICON,
                Some(windows::Win32::Foundation::WPARAM(
                    windows::Win32::UI::WindowsAndMessaging::ICON_BIG as usize,
                )),
                Some(windows::Win32::Foundation::LPARAM(icon.0 as isize)),
            );
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                window,
                windows::Win32::UI::WindowsAndMessaging::WM_SETICON,
                Some(windows::Win32::Foundation::WPARAM(
                    windows::Win32::UI::WindowsAndMessaging::ICON_SMALL as usize,
                )),
                Some(windows::Win32::Foundation::LPARAM(icon.0 as isize)),
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        Err(PlatformError::UnsupportedPlatform)
    }
}
pub fn fences_installed() -> bool {
    #[cfg(windows)]
    {
        let mut roots = Vec::new();
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(path) = env::var_os(variable) {
                roots.push(PathBuf::from(path));
            }
        }
        roots.sort();
        roots.dedup();
        roots.into_iter().any(|root| {
            let stardock = root.join("Stardock");
            if stardock.join("Fences").join("Fences.exe").is_file() {
                return true;
            }
            let Ok(entries) = fs::read_dir(stardock) else {
                return false;
            };
            entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                name.starts_with("fences") && entry.path().join("Fences.exe").is_file()
            })
        })
    }
    #[cfg(not(windows))]
    {
        false
    }
}
pub fn run_as_admin_and_wait(executable: &Path, args: &[String]) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        let quote = |value: &str| format!("'{}'", value.replace("'", "''"));
        let executable = quote(&executable.display().to_string());
        let arguments = args
            .iter()
            .map(|argument| quote(argument))
            .collect::<Vec<_>>()
            .join(", ");
        let script = format!(
            "$process = Start-Process -FilePath {executable} -ArgumentList @({arguments}) -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $process.ExitCode"
        );
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .status()
            .map_err(|error| PlatformError::Elevation(error.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::Elevation(format!(
                "管理员恢复进程返回码 {:?}",
                status.code()
            )))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (executable, args);
        Err(PlatformError::UnsupportedPlatform)
    }
}
pub fn open_in_explorer(path: &Path) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| PlatformError::OpenPath(error.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(PlatformError::UnsupportedPlatform)
    }
}
pub fn unregister_backup_task() -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        let status = Command::new("schtasks.exe")
            .args(["/Delete", "/TN", "DesktopSnapshot\\自动备份", "/F"])
            .status()
            .map_err(|error| PlatformError::TaskScheduler(error.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::TaskScheduler(format!(
                "schtasks.exe 返回码 {:?}",
                status.code()
            )))
        }
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
fn known_folder_path(folder_id: &windows::core::GUID) -> Result<PathBuf, PlatformError> {
    let raw_path = unsafe {
        windows::Win32::UI::Shell::SHGetKnownFolderPath(
            folder_id,
            windows::Win32::UI::Shell::KF_FLAG_DEFAULT,
            None,
        )?
    };
    let mut length = 0;
    let path = unsafe {
        while *raw_path.0.add(length) != 0 {
            length += 1;
        }
        let wide = slice::from_raw_parts(raw_path.0, length);
        PathBuf::from(String::from_utf16_lossy(wide))
    };
    unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(raw_path.0 as *const _));
    }
    Ok(path)
}

pub fn data_directory() -> Result<PathBuf, PlatformError> {
    #[cfg(windows)]
    {
        let local_app_data =
            env::var_os("LOCALAPPDATA").ok_or(PlatformError::UnsupportedPlatform)?;
        Ok(PathBuf::from(local_app_data).join("DesktopSnapshot"))
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::UnsupportedPlatform)
    }
}

pub fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn shortcut_target(path: &Path) -> (Option<String>, Option<bool>) {
    let target = ShellLink::open(path, WINDOWS_1252)
        .ok()
        .and_then(|link| link.link_target());
    let exists = target
        .as_deref()
        .map(|value| expand_environment_variables(value).exists());
    (target, exists)
}

fn expand_environment_variables(value: &str) -> PathBuf {
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            result.push('%');
            result.push_str(after_start);
            break;
        };
        let name = &after_start[..end];
        if let Ok(expanded) = env::var(name) {
            result.push_str(&expanded);
        } else {
            result.push('%');
            result.push_str(name);
            result.push('%');
        }
        remaining = &after_start[end + 1..];
    }
    if !remaining.is_empty() {
        result.push_str(remaining);
    }
    PathBuf::from(result)
}

fn content_path(snapshot_dir: &Path, item: &DesktopItem) -> PathBuf {
    let root = match item.root {
        DesktopRoot::User => "user",
        DesktopRoot::Public => "public",
    };
    snapshot_dir
        .join("content")
        .join(root)
        .join(&item.relative_path)
}

fn copy_directory(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else {
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

fn merge_directory(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            merge_directory(&source_path, &target_path)?;
        } else if !target_path.exists() {
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}
fn classify(path: &Path, is_directory: bool) -> DesktopItemKind {
    if is_directory {
        return DesktopItemKind::Directory;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("lnk") => DesktopItemKind::Shortcut,
        Some(extension) if extension.eq_ignore_ascii_case("url") => DesktopItemKind::Url,
        Some(_) => DesktopItemKind::File,
        None => DesktopItemKind::Other,
    }
}
