use clap::{Parser, Subcommand};
use desktop_snapshot_core::{ChangeKind, DesktopLayout, compare_snapshots};
use desktop_snapshot_platform_windows::{
    backup_contents, capture_layout, capture_screenshots, data_directory, desktop_locations,
    now_unix_seconds, open_in_explorer, register_backup_task, restore_layout,
    restore_missing_contents, scan_desktop, unregister_backup_task,
};
use desktop_snapshot_storage::SnapshotStore;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "desktop-snapshot", version, about = "桌面项目和布局快照工具")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 扫描桌面、保存布局并备份桌面项目内容
    Backup {
        /// 仅输出错误和结果摘要
        #[arg(long)]
        quiet: bool,
    },
    /// 列出本地快照
    List,
    /// 将当前桌面与最近快照比较
    Check,
    /// 从快照恢复桌面项目和图标布局
    Restore {
        /// 快照 ID，默认使用最近快照
        #[arg(long)]
        snapshot: Option<String>,
        /// 只恢复指定的相对路径
        #[arg(long)]
        item: Option<String>,
        /// 目标已经存在时覆盖
        #[arg(long)]
        overwrite: bool,
        /// 只恢复项目，不恢复图标布局
        #[arg(long)]
        without_layout: bool,
    },
    /// 删除快照，默认要求交互确认
    Delete {
        /// 要删除的快照 ID
        #[arg(long)]
        snapshot: String,
        /// 跳过确认，直接删除
        #[arg(long)]
        yes: bool,
    },
    /// 在资源管理器中打开备份目录
    Open {
        /// 可选的快照 ID；不指定时打开 DesktopSnapshot 根目录
        #[arg(long)]
        snapshot: Option<String>,
    },
    /// 管理 Windows 自动备份任务
    Schedule {
        #[command(subcommand)]
        action: ScheduleCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ScheduleCommand {
    /// 注册或更新自动备份任务
    Install {
        /// 自动备份间隔，单位为分钟
        #[arg(long, default_value_t = 15)]
        interval_minutes: u32,
    },
    /// 删除自动备份任务
    Uninstall,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("错误：{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.command {
        Command::Backup { quiet } => backup(quiet)?,
        Command::List => list_snapshots()?,
        Command::Check => check_current_state()?,
        Command::Restore {
            snapshot,
            item,
            overwrite,
            without_layout,
        } => restore(
            snapshot.as_deref(),
            item.as_deref(),
            overwrite,
            !without_layout,
        )?,
        Command::Delete { snapshot, yes } => {
            delete_snapshot(&snapshot, yes)?;
        }
        Command::Open { snapshot } => {
            let store = open_store()?;
            let path = match snapshot.as_deref() {
                Some(snapshot_id) => store.snapshot_dir(snapshot_id),
                None => store.root().to_path_buf(),
            };
            open_in_explorer(&path)?;
            println!("已打开备份目录：{}", path.display());
        }
        Command::Schedule { action } => match action {
            ScheduleCommand::Install { interval_minutes } => {
                register_backup_task(&std::env::current_exe()?, interval_minutes)?;
                println!("已注册自动备份任务，间隔 {interval_minutes} 分钟。");
            }
            ScheduleCommand::Uninstall => {
                unregister_backup_task()?;
                println!("已删除自动备份任务。");
            }
        },
    }
    Ok(())
}

fn backup(quiet: bool) -> Result<(), Box<dyn std::error::Error>> {
    let locations = desktop_locations()?;
    let mut items = scan_desktop()?;
    let layout = capture_layout(&locations, &items).ok();
    let store = open_store()?;
    let snapshot_id = store.create_snapshot(now_unix_seconds(), &items, layout.clone())?;
    let content_report = backup_contents(&store.snapshot_dir(&snapshot_id), &locations, &mut items);
    store.update_snapshot(&snapshot_id, &items, layout)?;
    let screenshot_paths = match capture_screenshots(&store.snapshot_dir(&snapshot_id)) {
        Ok(paths) => paths,
        Err(error) => {
            if !quiet {
                eprintln!("截图保存失败：{error}");
            }
            Vec::new()
        }
    };
    store.update_screenshots(&snapshot_id, screenshot_paths.clone())?;

    if !quiet {
        println!(
            "已创建快照 #{snapshot_id}，记录 {} 个桌面项目，成功备份 {} 个项目。",
            items.len(),
            content_report.backed_up
        );
        if !content_report.failed.is_empty() {
            println!("以下项目内容备份失败：");
            for item in content_report.failed {
                println!("  - {item}");
            }
        }
    }
    Ok(())
}

fn delete_snapshot(
    snapshot_id: &str,
    skip_confirmation: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = open_store()?;
    let manifest = store.load_manifest(snapshot_id)?;
    if !skip_confirmation {
        println!(
            "即将删除快照 #{}（{}，{} 个项目）。此操作不可撤销。",
            manifest.id, manifest.title, manifest.item_count
        );
        print!("确认删除？输入 y/yes 继续，其他输入取消：");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("已取消删除。");
            return Ok(());
        }
    }
    let deleted = store.delete_snapshot(snapshot_id)?;
    println!("已删除快照 #{}。", deleted.id);
    Ok(())
}
fn list_snapshots() -> Result<(), Box<dyn std::error::Error>> {
    let snapshots = open_store()?.list_snapshots()?;
    if snapshots.is_empty() {
        println!("暂无快照。");
    } else {
        for snapshot in snapshots {
            println!(
                "{} | {} | 时间戳={} | 项目数={}",
                snapshot.id, snapshot.title, snapshot.created_unix_seconds, snapshot.item_count
            );
        }
    }
    Ok(())
}

fn check_current_state() -> Result<(), Box<dyn std::error::Error>> {
    let store = open_store()?;
    let Some(snapshot_id) = store.latest_snapshot_id()? else {
        println!("暂无快照，请先执行 backup。");
        return Ok(());
    };
    let previous = store.load_items(&snapshot_id)?;
    let current = scan_desktop()?;
    let changes = compare_snapshots(&previous, &current);
    if changes.is_empty() {
        println!("当前桌面与快照 #{snapshot_id} 一致。");
    } else {
        for change in changes {
            let action = match change.kind {
                ChangeKind::Added => "新增",
                ChangeKind::Removed => "删除",
                ChangeKind::Modified => "修改",
            };
            println!(
                "{action}: [{:?}] {}",
                change.item.root, change.item.relative_path
            );
        }
    }
    for item in current {
        if item.target_exists == Some(false) {
            println!(
                "警告：快捷方式 {} 的目标不存在：{}",
                item.relative_path,
                item.target_path.as_deref().unwrap_or("未知")
            );
        }
    }
    Ok(())
}

fn restore(
    snapshot_id: Option<&str>,
    only_item: Option<&str>,
    overwrite: bool,
    restore_positions: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let locations = desktop_locations()?;
    let store = open_store()?;
    let snapshot_id = match snapshot_id {
        Some(value) => value.to_owned(),
        None => store
            .latest_snapshot_id()?
            .ok_or("暂无快照，请先执行 backup")?,
    };
    let manifest = store.load_manifest(&snapshot_id)?;
    let report = restore_missing_contents(
        &store.snapshot_dir(&snapshot_id),
        &locations,
        &manifest.items,
        only_item,
        overwrite,
    );
    println!("已恢复项目：{}", report.restored.len());
    println!("跳过项目：{}", report.skipped.len());
    println!("失败项目：{}", report.failed.len());

    if restore_positions {
        if let Some(mut layout) = manifest.layout {
            if let Some(item) = only_item {
                layout.icons.retain(|icon| icon.relative_path == item);
            }
            if let Err(error) = restore_layout(&locations, &layout) {
                eprintln!("布局恢复失败：{error}");
            } else {
                println!("图标布局已恢复。");
            }
        } else {
            println!("该快照没有图标布局信息。");
        }
    }
    Ok(())
}

fn open_store() -> Result<SnapshotStore, Box<dyn std::error::Error>> {
    let directory: PathBuf = data_directory()?;
    fs::create_dir_all(&directory)?;
    Ok(SnapshotStore::open(directory)?)
}

#[allow(dead_code)]
fn _layout_is_kept_for_manifest(_: Option<DesktopLayout>) {}
