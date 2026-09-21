# DesktopSnapshot

DesktopSnapshot 是一个面向 Windows 的本地桌面快照与恢复工具。

项目目标是同时保存：

- 桌面项目本身（快捷方式、网址、文件和文件夹）；
- `.lnk` 的目标路径及目标是否存在；
- 桌面项目的本地内容备份；
- 桌面图标布局和多显示器边界；
- 删除项目的历史快照。

项目不上传文件，不依赖云服务，也不要求后台托盘常驻。自动备份计划通过 Windows 任务计划程序调用无界面 CLI 完成。

## 当前状态

当前版本已经实现第一版完整链路：

- Rust workspace；
- `desktop-snapshot` 无界面 CLI；
- 通过 Windows Known Folder API 扫描重定向后的用户桌面和公共桌面；
- `.lnk` 目标路径解析和目标存在性检测；
- `.lnk`、`.url`、文件、文件夹的基础分类；
- 本地快照目录和 JSON 清单存储；
- 桌面项目内容备份和缺失项目恢复；
- Windows Shell COM 图标位置读取和恢复；
- 显示器名称、边界和主显示器信息保存；
- `backup`、`list`、`check`、`restore` 命令；
- Windows 任务计划自动备份注册和删除；
- Tauri + Vue + Element Plus 快照列表、创建、恢复和任务注册界面；
- 快照保存主显示器截图并支持缩略图和大图预览；
- 无控制台窗口的 GUI 可执行文件；
- 内置 DesktopSnapshot 蓝色桌面快照图标；
- 核心快照差异测试。

当前已知限制：

- 系统特殊图标（如回收站、此电脑）还没有单独的恢复策略；
- 显示器拓扑变化后的坐标映射还未做高级适配；
- GUI 已支持表格查看、选择、恢复和删除快照；
- 内容备份当前采用每个快照独立复制，尚未做去重和保留策略；
- 定时任务注册已经支持 CLI，但安装程序还没有自动注册流程。

## 构建

需要安装 Rust stable 和 Windows MSVC 工具链。

```powershell
npm ci --prefix crates/ui
npm run build --prefix crates/ui
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo build --release --locked -p desktop-snapshot-cli
npm run tauri build --prefix crates/ui
```

GUI 使用 Tauri + Vue 构建，图标通过 Tauri 配置嵌入 Windows 安装包；双击运行时不会额外打开 CMD 控制台窗口。

## CLI 用法

```powershell
# 创建快照、备份项目内容和图标布局
cargo run -p desktop-snapshot-cli -- backup

# 适合任务计划调用
cargo run -p desktop-snapshot-cli -- backup --quiet

# 查看快照
cargo run -p desktop-snapshot-cli -- list

# 检查当前桌面变化和失效快捷方式
cargo run -p desktop-snapshot-cli -- check

# 恢复最近快照
cargo run -p desktop-snapshot-cli -- restore

# 恢复指定快照，不覆盖现有项目
cargo run -p desktop-snapshot-cli -- restore --snapshot <snapshot-id>

# 只恢复一个项目
cargo run -p desktop-snapshot-cli -- restore --snapshot <snapshot-id> --item "微信.lnk"

# 恢复时覆盖现有项目，并且不恢复图标位置
cargo run -p desktop-snapshot-cli -- restore --snapshot <snapshot-id> --overwrite --without-layout

# 注册或删除每 15 分钟一次的自动备份任务
cargo run -p desktop-snapshot-cli -- schedule install --interval-minutes 15
cargo run -p desktop-snapshot-cli -- schedule uninstall
```

## 快照目录

本地快照默认保存在：

```text
%LOCALAPPDATA%\DesktopSnapshot\snapshots\<snapshot-id>\
```

每个快照目录包含：

```text
manifest.json   # 标题、路径、时间、快捷方式状态和布局信息
content\        # 用户桌面和公共桌面的项目内容
```

快照清单中还会保存：

- 显示器边界和主显示器标记；
- 图标相对于桌面视图的位置；
- 文件是否已经复制到快照内容目录；
- 快捷方式目标路径和目标存在状态。

## 许可证

本项目使用 GPL-3.0-only 许可证，详见 [LICENSE](LICENSE)。

图形界面使用 Tauri、Vue 和 Vite；Rust 后端继续使用 GPL-3.0-only，第三方依赖说明见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
