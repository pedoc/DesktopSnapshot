use desktop_snapshot_core::{DesktopItem, DesktopLayout};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("文件系统错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("快照清单序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("快照不存在: {0}")]
    SnapshotNotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub created_unix_seconds: i64,
    pub item_count: usize,
    pub items: Vec<DesktopItem>,
    #[serde(default)]
    pub layout: Option<DesktopLayout>,
    #[serde(default)]
    pub screenshots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub id: String,
    pub title: String,
    pub created_unix_seconds: i64,
    pub item_count: usize,
}

pub struct SnapshotStore {
    root: PathBuf,
    snapshots_dir: PathBuf,
}

fn is_safe_snapshot_id(snapshot_id: &str) -> bool {
    !snapshot_id.is_empty()
        && snapshot_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
}
impl SnapshotStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let root = path.as_ref().to_path_buf();
        let snapshots_dir = root.join("snapshots");
        fs::create_dir_all(&snapshots_dir)?;
        Ok(Self {
            root,
            snapshots_dir,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create_snapshot(
        &self,
        created_unix_seconds: i64,
        items: &[DesktopItem],
        layout: Option<DesktopLayout>,
    ) -> Result<String, StorageError> {
        let id = self.next_snapshot_id(created_unix_seconds)?;
        let snapshot_dir = self.snapshots_dir.join(&id);
        let content_dir = snapshot_dir.join("content");
        fs::create_dir_all(content_dir.join("user"))?;
        fs::create_dir_all(content_dir.join("public"))?;

        let manifest = SnapshotManifest {
            schema_version: 1,
            title: format!("自动快照 {id}"),
            id: id.clone(),
            created_unix_seconds,
            item_count: items.len(),
            items: items.to_vec(),
            layout,
            screenshots: Vec::new(),
        };
        let manifest_path = snapshot_dir.join("manifest.json");
        let temporary_manifest = snapshot_dir.join("manifest.json.tmp");
        fs::write(&temporary_manifest, serde_json::to_vec_pretty(&manifest)?)?;
        fs::rename(temporary_manifest, manifest_path)?;
        Ok(id)
    }

    pub fn snapshot_dir(&self, snapshot_id: &str) -> PathBuf {
        self.snapshots_dir.join(snapshot_id)
    }

    pub fn load_manifest(&self, snapshot_id: &str) -> Result<SnapshotManifest, StorageError> {
        self.read_manifest(snapshot_id)
    }

    pub fn update_screenshots(
        &self,
        snapshot_id: &str,
        screenshots: Vec<String>,
    ) -> Result<(), StorageError> {
        let mut manifest = self.read_manifest(snapshot_id)?;
        manifest.screenshots = screenshots;
        self.write_manifest(&manifest)
    }
    pub fn update_snapshot(
        &self,
        snapshot_id: &str,
        items: &[DesktopItem],
        layout: Option<DesktopLayout>,
    ) -> Result<(), StorageError> {
        let mut manifest = self.read_manifest(snapshot_id)?;
        manifest.item_count = items.len();
        manifest.items = items.to_vec();
        manifest.layout = layout;
        self.write_manifest(&manifest)
    }
    pub fn rename_snapshot(
        &self,
        snapshot_id: &str,
        title: String,
    ) -> Result<SnapshotSummary, StorageError> {
        let title = title.trim().to_owned();
        if title.is_empty() || title.len() > 120 {
            return Err(StorageError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        let mut manifest = self.read_manifest(snapshot_id)?;
        manifest.title = title;
        self.write_manifest(&manifest)?;
        Ok(SnapshotSummary {
            id: manifest.id,
            title: manifest.title,
            created_unix_seconds: manifest.created_unix_seconds,
            item_count: manifest.item_count,
        })
    }
    pub fn delete_snapshot(&self, snapshot_id: &str) -> Result<SnapshotSummary, StorageError> {
        if !is_safe_snapshot_id(snapshot_id) {
            return Err(StorageError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        let manifest = self.read_manifest(snapshot_id)?;
        let summary = SnapshotSummary {
            id: manifest.id,
            title: manifest.title,
            created_unix_seconds: manifest.created_unix_seconds,
            item_count: manifest.item_count,
        };
        fs::remove_dir_all(self.snapshot_dir(snapshot_id))?;
        Ok(summary)
    }
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotSummary>, StorageError> {
        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&self.snapshots_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let manifest = self.read_manifest(&entry.file_name().to_string_lossy())?;
            snapshots.push(SnapshotSummary {
                id: manifest.id,
                title: manifest.title,
                created_unix_seconds: manifest.created_unix_seconds,
                item_count: manifest.item_count,
            });
        }
        snapshots.sort_by(|left, right| {
            right
                .created_unix_seconds
                .cmp(&left.created_unix_seconds)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(snapshots)
    }

    pub fn latest_snapshot_id(&self) -> Result<Option<String>, StorageError> {
        Ok(self
            .list_snapshots()?
            .into_iter()
            .next()
            .map(|summary| summary.id))
    }

    pub fn load_items(&self, snapshot_id: &str) -> Result<Vec<DesktopItem>, StorageError> {
        Ok(self.read_manifest(snapshot_id)?.items)
    }

    fn next_snapshot_id(&self, created_unix_seconds: i64) -> Result<String, StorageError> {
        let base = format!("{created_unix_seconds}");
        let mut candidate = base.clone();
        let mut suffix = 2;
        while self.snapshots_dir.join(&candidate).exists() {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        Ok(candidate)
    }

    fn read_manifest(&self, snapshot_id: &str) -> Result<SnapshotManifest, StorageError> {
        let manifest_path = self.snapshot_dir(snapshot_id).join("manifest.json");
        if !manifest_path.exists() {
            return Err(StorageError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        Ok(serde_json::from_slice(&fs::read(manifest_path)?)?)
    }

    fn write_manifest(&self, manifest: &SnapshotManifest) -> Result<(), StorageError> {
        let snapshot_dir = self.snapshot_dir(&manifest.id);
        let manifest_path = snapshot_dir.join("manifest.json");
        let temporary_manifest = snapshot_dir.join("manifest.json.tmp");
        fs::write(&temporary_manifest, serde_json::to_vec_pretty(manifest)?)?;
        fs::rename(temporary_manifest, manifest_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn deletes_snapshot_directory() {
        let root = std::env::temp_dir().join(format!(
            "desktop-snapshot-storage-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = SnapshotStore::open(&root).unwrap();
        let snapshot_id = store.create_snapshot(1, &[], None).unwrap();
        assert!(store.snapshot_dir(&snapshot_id).exists());
        store.delete_snapshot(&snapshot_id).unwrap();
        assert!(!store.snapshot_dir(&snapshot_id).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
