use crate::{DesktopLocation, PlatformError};
use desktop_snapshot_core::{DesktopItem, DesktopLayout, IconPosition, MonitorLayout};
use std::mem::size_of;
use std::path::Path;
use std::ptr::null_mut;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    FWF_AUTOARRANGE, IFolderView, IFolderView2, IShellBrowser, IShellFolder, IShellWindows,
    SID_STopLevelBrowser, SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::core::{BOOL, Interface, PCWSTR};

struct ComGuard;

impl ComGuard {
    fn initialize() -> Result<Self, PlatformError> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn capture_layout(
    _locations: &[DesktopLocation],
    items: &[DesktopItem],
) -> Result<DesktopLayout, PlatformError> {
    let monitors = enumerate_monitors()?;
    let icons = with_desktop_view(|view| {
        let mut positions = Vec::new();
        for item in items {
            if let Ok(point) = item_position(view, &item.relative_path) {
                positions.push(IconPosition {
                    root: item.root,
                    relative_path: item.relative_path.clone(),
                    x: point.x,
                    y: point.y,
                });
            }
        }
        Ok(positions)
    })?;
    let auto_arrange = with_desktop_view(|view| Ok(unsafe { view.GetAutoArrange().is_ok() }))?;
    Ok(DesktopLayout {
        monitors,
        icons,
        auto_arrange: Some(auto_arrange),
    })
}

pub fn restore_layout(
    locations: &[DesktopLocation],
    layout: &DesktopLayout,
) -> Result<(), PlatformError> {
    with_desktop_view(|view| {
        let folder_view2 = view.cast::<IFolderView2>().ok();
        if let Some(folder_view2) = &folder_view2 {
            unsafe {
                folder_view2.SetCurrentFolderFlags(FWF_AUTOARRANGE.0 as u32, 0)?;
            }
        }
        for icon in &layout.icons {
            let Some(location) = locations.iter().find(|location| location.root == icon.root)
            else {
                continue;
            };
            let path = location.path.join(&icon.relative_path);
            if !path.exists() {
                continue;
            }
            let pidl = match parse_pidl(view, &icon.relative_path) {
                Ok(pidl) => pidl,
                Err(_) => continue,
            };
            let point = POINT {
                x: icon.x,
                y: icon.y,
            };
            let pidls = [pidl as *const ITEMIDLIST];
            let result = unsafe {
                view.SelectAndPositionItems(
                    1,
                    pidls.as_ptr(),
                    Some(&point),
                    SVSI_POSITIONITEM.0 as u32,
                )
            };
            unsafe {
                CoTaskMemFree(Some(pidl.cast()));
            }
            result?;
        }
        Ok(())
    })
}

fn with_desktop_view<T>(
    callback: impl FnOnce(&IFolderView) -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let _com = ComGuard::initialize()?;
    let folder_view = unsafe {
        let shell_windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let location = VARIANT::default();
        let root = VARIANT::default();
        let mut hwnd = 0;
        let dispatch = shell_windows.FindWindowSW(
            &location,
            &root,
            SWC_DESKTOP,
            &mut hwnd,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let shell_view = browser.QueryActiveShellView()?;
        shell_view.cast::<IFolderView>()?
    };
    callback(&folder_view)
}

fn item_position(view: &IFolderView, relative_path: &str) -> Result<POINT, PlatformError> {
    let pidl = parse_pidl(view, relative_path)?;
    let result = unsafe { view.GetItemPosition(pidl as *const ITEMIDLIST) };
    unsafe {
        CoTaskMemFree(Some(pidl.cast()));
    }
    Ok(result?)
}

fn parse_pidl(view: &IFolderView, relative_path: &str) -> Result<*mut ITEMIDLIST, PlatformError> {
    let wide: Vec<u16> = relative_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut pidl = null_mut();
    let mut attributes = 0;
    let desktop_folder: IShellFolder = unsafe { view.GetFolder()? };
    unsafe {
        desktop_folder.ParseDisplayName(
            HWND(null_mut()),
            None,
            PCWSTR(wide.as_ptr()),
            None,
            &mut pidl,
            &mut attributes,
        )?;
    }
    Ok(pidl)
}

fn enumerate_monitors() -> Result<Vec<MonitorLayout>, PlatformError> {
    let mut monitors = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_callback),
            LPARAM(&mut monitors as *mut Vec<MonitorLayout> as isize),
        )
        .ok()?;
    }
    Ok(monitors)
}

unsafe extern "system" fn monitor_callback(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<MonitorLayout>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let result = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if !result.as_bool() {
        return BOOL(1);
    }
    let device_end = info
        .szDevice
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(info.szDevice.len());
    let rect = info.monitorInfo.rcMonitor;
    monitors.push(MonitorLayout {
        device_name: String::from_utf16_lossy(&info.szDevice[..device_end]),
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        primary: info.monitorInfo.dwFlags == 1,
    });
    BOOL(1)
}

#[allow(dead_code)]
fn _path_is_used(path: &Path) -> bool {
    path.exists()
}
