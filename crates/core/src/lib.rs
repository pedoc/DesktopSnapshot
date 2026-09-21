use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum DesktopRoot {
    User,
    Public,
}

impl DesktopRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Public => "public",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DesktopItemKind {
    File,
    Directory,
    Shortcut,
    Url,
    Other,
}

impl DesktopItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Shortcut => "shortcut",
            Self::Url => "url",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopItem {
    pub root: DesktopRoot,
    pub relative_path: String,
    pub kind: DesktopItemKind,
    pub size: u64,
    pub modified_unix_seconds: i64,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub target_exists: Option<bool>,
    #[serde(default)]
    pub content_backed_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopLayout {
    pub monitors: Vec<MonitorLayout>,
    pub icons: Vec<IconPosition>,
    #[serde(default)]
    pub auto_arrange: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorLayout {
    pub device_name: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IconPosition {
    pub root: DesktopRoot,
    pub relative_path: String,
    pub x: i32,
    pub y: i32,
}

impl DesktopItem {
    pub fn key(&self) -> (DesktopRoot, &str) {
        (self.root, self.relative_path.as_str())
    }

    pub fn same_desktop_state(&self, other: &Self) -> bool {
        self.root == other.root
            && self.relative_path == other.relative_path
            && self.kind == other.kind
            && self.size == other.size
            && self.modified_unix_seconds == other.modified_unix_seconds
            && self.target_path == other.target_path
            && self.target_exists == other.target_exists
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopChange<'a> {
    pub kind: ChangeKind,
    pub item: &'a DesktopItem,
}

pub fn compare_snapshots<'a>(
    previous: &'a [DesktopItem],
    current: &'a [DesktopItem],
) -> Vec<DesktopChange<'a>> {
    let previous_by_key: BTreeMap<_, _> = previous.iter().map(|item| (item.key(), item)).collect();
    let current_by_key: BTreeMap<_, _> = current.iter().map(|item| (item.key(), item)).collect();
    let mut changes = Vec::new();
    let all_keys: BTreeSet<_> = previous_by_key
        .keys()
        .chain(current_by_key.keys())
        .copied()
        .collect();

    for key in all_keys {
        match (previous_by_key.get(&key), current_by_key.get(&key)) {
            (None, Some(item)) => changes.push(DesktopChange {
                kind: ChangeKind::Added,
                item,
            }),
            (Some(item), None) => changes.push(DesktopChange {
                kind: ChangeKind::Removed,
                item,
            }),
            (Some(previous), Some(current)) if !previous.same_desktop_state(current) => changes
                .push(DesktopChange {
                    kind: ChangeKind::Modified,
                    item: current,
                }),
            _ => {}
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, size: u64) -> DesktopItem {
        DesktopItem {
            root: DesktopRoot::User,
            relative_path: path.to_owned(),
            kind: DesktopItemKind::File,
            size,
            modified_unix_seconds: 1,
            target_path: None,
            target_exists: None,
            content_backed_up: false,
        }
    }

    #[test]
    fn ignores_content_backup_status_when_comparing() {
        let mut previous = item("same.txt", 1);
        previous.content_backed_up = true;
        let current = item("same.txt", 1);
        assert!(compare_snapshots(&[previous], &[current]).is_empty());
    }
    #[test]
    fn detects_added_removed_and_modified_items() {
        let previous = vec![item("old.txt", 1), item("changed.txt", 1)];
        let current = vec![item("new.txt", 1), item("changed.txt", 2)];
        let changes = compare_snapshots(&previous, &current);
        assert_eq!(changes.len(), 3);
        assert!(
            changes
                .iter()
                .any(|change| change.kind == ChangeKind::Added)
        );
        assert!(
            changes
                .iter()
                .any(|change| change.kind == ChangeKind::Removed)
        );
        assert!(
            changes
                .iter()
                .any(|change| change.kind == ChangeKind::Modified)
        );
    }
}
