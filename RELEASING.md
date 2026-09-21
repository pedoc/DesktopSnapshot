# 发布流程

GitHub Actions 已配置自动发布 Windows x64 Release。

## CI

以下事件会触发 `.github/workflows/ci.yml`：

- 推送到 `master`；
- 针对 `master` 的 Pull Request。

CI 会执行：

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
npm ci --prefix crates/ui
npm run tauri build --prefix crates/ui
cargo build --release --locked -p desktop-snapshot-cli
```

## 自动发布

推送 `v` 开头的 Git tag 后，会触发 `.github/workflows/release.yml`：

```powershell
git tag v0.1.0
git push origin v0.1.0
```

Workflow 会：

1. 在 Windows runner 上构建 CLI 和 Tauri Windows 安装包；
2. 打包 `desktop-snapshot.exe`、NSIS 安装包和 MSI 安装包；
3. 附带 `README.md`、`LICENSE` 和第三方声明；
4. 生成 `SHA256SUMS.txt`；
5. 自动创建 GitHub Release 并上传 ZIP 包。

创建仓库并推送代码后，只要仓库允许 GitHub Actions 写入 Contents，推送版本 tag 就会自动发布。
