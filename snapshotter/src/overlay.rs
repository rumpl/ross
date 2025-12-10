use crate::error::SnapshotterError;
use crate::types::{Mount, SnapshotInfo, SnapshotKind, Usage};
use flate2::read::GzDecoder;
use ross_store::FileSystemStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::Archive;
use tokio::fs;
use tokio::sync::RwLock;

const SNAPSHOTS_DIR: &str = "snapshots";
const METADATA_FILE: &str = "metadata.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotMetadata {
    info: SnapshotInfo,
}

pub struct OverlaySnapshotter {
    root: PathBuf,
    store: Arc<FileSystemStore>,
    snapshots: RwLock<HashMap<String, SnapshotInfo>>,
}

impl OverlaySnapshotter {
    pub async fn new(
        root: impl AsRef<Path>,
        store: Arc<FileSystemStore>,
    ) -> Result<Self, SnapshotterError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).await?;
        fs::create_dir_all(root.join(SNAPSHOTS_DIR)).await?;

        let snapshotter = Self {
            root,
            store,
            snapshots: RwLock::new(HashMap::new()),
        };

        snapshotter.load_snapshots().await?;

        Ok(snapshotter)
    }

    async fn load_snapshots(&self) -> Result<(), SnapshotterError> {
        let snapshots_dir = self.root.join(SNAPSHOTS_DIR);
        let mut snapshots = self.snapshots.write().await;

        let mut entries = match fs::read_dir(&snapshots_dir).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }

            let meta_path = entry.path().join(METADATA_FILE);
            if !meta_path.exists() {
                continue;
            }

            let content = fs::read_to_string(&meta_path).await?;
            let metadata: SnapshotMetadata = serde_json::from_str(&content)?;
            snapshots.insert(metadata.info.key.clone(), metadata.info);
        }

        Ok(())
    }

    fn snapshot_dir(&self, key: &str) -> PathBuf {
        self.root.join(SNAPSHOTS_DIR).join(sanitize_key(key))
    }

    fn fs_dir(&self, key: &str) -> PathBuf {
        self.snapshot_dir(key).join("fs")
    }

    fn work_dir(&self, key: &str) -> PathBuf {
        self.snapshot_dir(key).join("work")
    }

    async fn save_metadata(&self, info: &SnapshotInfo) -> Result<(), SnapshotterError> {
        let dir = self.snapshot_dir(&info.key);
        fs::create_dir_all(&dir).await?;

        let metadata = SnapshotMetadata { info: info.clone() };
        let content = serde_json::to_string_pretty(&metadata)?;
        fs::write(dir.join(METADATA_FILE), content).await?;

        Ok(())
    }

    fn get_parent_chain(
        &self,
        snapshots: &HashMap<String, SnapshotInfo>,
        key: &str,
    ) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = Some(key.to_string());

        while let Some(k) = current {
            if let Some(info) = snapshots.get(&k) {
                chain.push(k);
                current = info.parent.clone();
            } else {
                break;
            }
        }

        chain
    }

    fn build_overlay_mounts(
        &self,
        key: &str,
        parent_chain: &[String],
        readonly: bool,
    ) -> Vec<Mount> {
        if parent_chain.is_empty() {
            return vec![Mount {
                mount_type: "bind".to_string(),
                source: self.fs_dir(key).to_string_lossy().to_string(),
                target: String::new(),
                options: if readonly {
                    vec!["ro".to_string(), "rbind".to_string()]
                } else {
                    vec!["rw".to_string(), "rbind".to_string()]
                },
            }];
        }

        let lower_dirs: Vec<String> = parent_chain
            .iter()
            .map(|k| self.fs_dir(k).to_string_lossy().to_string())
            .collect();

        let mut options = vec![format!("lowerdir={}", lower_dirs.join(":"))];

        if !readonly {
            options.push(format!("upperdir={}", self.fs_dir(key).to_string_lossy()));
            options.push(format!("workdir={}", self.work_dir(key).to_string_lossy()));
        }

        vec![Mount {
            mount_type: "overlay".to_string(),
            source: "overlay".to_string(),
            target: String::new(),
            options,
        }]
    }

    pub async fn prepare(
        &self,
        key: &str,
        parent: Option<&str>,
        labels: HashMap<String, String>,
    ) -> Result<Vec<Mount>, SnapshotterError> {
        let mut snapshots = self.snapshots.write().await;

        if snapshots.contains_key(key) {
            return Err(SnapshotterError::AlreadyExists(key.to_string()));
        }

        if let Some(p) = parent {
            let parent_info = snapshots
                .get(p)
                .ok_or_else(|| SnapshotterError::ParentNotFound(p.to_string()))?;

            if parent_info.kind != SnapshotKind::Committed {
                return Err(SnapshotterError::InvalidState {
                    expected: "committed".to_string(),
                    actual: parent_info.kind.to_string(),
                });
            }
        }

        let snapshot_dir = self.snapshot_dir(key);
        fs::create_dir_all(&snapshot_dir).await?;
        fs::create_dir_all(self.fs_dir(key)).await?;
        fs::create_dir_all(self.work_dir(key)).await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let info = SnapshotInfo {
            key: key.to_string(),
            parent: parent.map(String::from),
            kind: SnapshotKind::Active,
            created_at: now,
            updated_at: now,
            labels,
        };

        self.save_metadata(&info).await?;
        snapshots.insert(key.to_string(), info);

        let parent_chain = parent
            .map(|p| self.get_parent_chain(&snapshots, p))
            .unwrap_or_default();

        Ok(self.build_overlay_mounts(key, &parent_chain, false))
    }

    pub async fn view(
        &self,
        key: &str,
        parent: Option<&str>,
        labels: HashMap<String, String>,
    ) -> Result<Vec<Mount>, SnapshotterError> {
        let mut snapshots = self.snapshots.write().await;

        if snapshots.contains_key(key) {
            return Err(SnapshotterError::AlreadyExists(key.to_string()));
        }

        if let Some(p) = parent {
            let parent_info = snapshots
                .get(p)
                .ok_or_else(|| SnapshotterError::ParentNotFound(p.to_string()))?;

            if parent_info.kind != SnapshotKind::Committed {
                return Err(SnapshotterError::InvalidState {
                    expected: "committed".to_string(),
                    actual: parent_info.kind.to_string(),
                });
            }
        }

        let snapshot_dir = self.snapshot_dir(key);
        fs::create_dir_all(&snapshot_dir).await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let info = SnapshotInfo {
            key: key.to_string(),
            parent: parent.map(String::from),
            kind: SnapshotKind::View,
            created_at: now,
            updated_at: now,
            labels,
        };

        self.save_metadata(&info).await?;
        snapshots.insert(key.to_string(), info);

        let parent_chain = parent
            .map(|p| self.get_parent_chain(&snapshots, p))
            .unwrap_or_default();

        Ok(self.build_overlay_mounts(key, &parent_chain, true))
    }

    pub async fn mounts(&self, key: &str) -> Result<Vec<Mount>, SnapshotterError> {
        let snapshots = self.snapshots.read().await;

        let info = snapshots
            .get(key)
            .ok_or_else(|| SnapshotterError::NotFound(key.to_string()))?;

        let readonly = info.kind == SnapshotKind::View || info.kind == SnapshotKind::Committed;

        let parent_chain = info
            .parent
            .as_ref()
            .map(|p| self.get_parent_chain(&snapshots, p))
            .unwrap_or_default();

        Ok(self.build_overlay_mounts(key, &parent_chain, readonly))
    }

    pub async fn commit(
        &self,
        key: &str,
        active_key: &str,
        labels: HashMap<String, String>,
    ) -> Result<(), SnapshotterError> {
        let mut snapshots = self.snapshots.write().await;

        if snapshots.contains_key(key) {
            return Err(SnapshotterError::AlreadyExists(key.to_string()));
        }

        let active_info = snapshots
            .get(active_key)
            .ok_or_else(|| SnapshotterError::NotFound(active_key.to_string()))?
            .clone();

        if active_info.kind != SnapshotKind::Active {
            return Err(SnapshotterError::InvalidState {
                expected: "active".to_string(),
                actual: active_info.kind.to_string(),
            });
        }

        let active_dir = self.snapshot_dir(active_key);
        let committed_dir = self.snapshot_dir(key);

        fs::rename(&active_dir, &committed_dir).await?;

        snapshots.remove(active_key);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut new_labels = active_info.labels;
        new_labels.extend(labels);

        let info = SnapshotInfo {
            key: key.to_string(),
            parent: active_info.parent,
            kind: SnapshotKind::Committed,
            created_at: active_info.created_at,
            updated_at: now,
            labels: new_labels,
        };

        self.save_metadata(&info).await?;
        snapshots.insert(key.to_string(), info);

        Ok(())
    }

    pub async fn remove(&self, key: &str) -> Result<(), SnapshotterError> {
        let mut snapshots = self.snapshots.write().await;

        if !snapshots.contains_key(key) {
            return Err(SnapshotterError::NotFound(key.to_string()));
        }

        let has_dependents = snapshots
            .values()
            .any(|info| info.parent.as_deref() == Some(key));

        if has_dependents {
            return Err(SnapshotterError::HasDependents(key.to_string()));
        }

        let snapshot_dir = self.snapshot_dir(key);
        if snapshot_dir.exists() {
            fs::remove_dir_all(&snapshot_dir).await?;
        }

        snapshots.remove(key);

        Ok(())
    }

    pub async fn stat(&self, key: &str) -> Result<SnapshotInfo, SnapshotterError> {
        let snapshots = self.snapshots.read().await;

        snapshots
            .get(key)
            .cloned()
            .ok_or_else(|| SnapshotterError::NotFound(key.to_string()))
    }

    pub async fn list(
        &self,
        parent_filter: Option<&str>,
    ) -> Result<Vec<SnapshotInfo>, SnapshotterError> {
        let snapshots = self.snapshots.read().await;

        let result: Vec<SnapshotInfo> = snapshots
            .values()
            .filter(|info| {
                parent_filter
                    .map(|p| info.parent.as_deref() == Some(p))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        Ok(result)
    }

    pub async fn usage(&self, key: &str) -> Result<Usage, SnapshotterError> {
        let snapshots = self.snapshots.read().await;

        if !snapshots.contains_key(key) {
            return Err(SnapshotterError::NotFound(key.to_string()));
        }

        let fs_dir = self.fs_dir(key);
        let (size, inodes) = calculate_dir_usage(&fs_dir).await?;

        Ok(Usage { size, inodes })
    }

    pub async fn cleanup(&self) -> Result<i64, SnapshotterError> {
        let mut reclaimed = 0i64;
        let snapshots_dir = self.root.join(SNAPSHOTS_DIR);
        let snapshots = self.snapshots.read().await;

        let mut entries = fs::read_dir(&snapshots_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();

            let known = snapshots
                .values()
                .any(|info| sanitize_key(&info.key) == name);

            if !known && entry.file_type().await?.is_dir() {
                let (size, _) = calculate_dir_usage(&entry.path()).await?;
                fs::remove_dir_all(entry.path()).await?;
                reclaimed += size;
            }
        }

        Ok(reclaimed)
    }

    pub async fn extract_layer(
        &self,
        digest: &str,
        parent_key: Option<&str>,
        key: &str,
        labels: HashMap<String, String>,
    ) -> Result<(String, i64), SnapshotterError> {
        let store_digest = parse_digest(digest)?;

        let blob_data = self
            .store
            .get_blob(&store_digest, 0, -1)
            .await
            .map_err(|e| {
                SnapshotterError::ExtractionFailed(format!("failed to get blob: {}", e))
            })?;

        let active_key = format!("{}-extract", key);
        self.prepare(&active_key, parent_key, HashMap::new())
            .await?;

        let extract_dir = self.fs_dir(&active_key);
        let size = extract_tar_gz(&blob_data, &extract_dir)?;

        let mut final_labels = labels;
        final_labels.insert(
            "containerd.io/snapshot/layer.digest".to_string(),
            digest.to_string(),
        );

        if let Err(e) = self.commit(key, &active_key, final_labels).await {
            let _ = self.remove(&active_key).await;
            return Err(e);
        }

        Ok((key.to_string(), size))
    }
}

fn sanitize_key(key: &str) -> String {
    key.replace(['/', ':'], "_")
}

fn parse_digest(digest: &str) -> Result<ross_store::Digest, SnapshotterError> {
    let parts: Vec<&str> = digest.split(':').collect();
    if parts.len() != 2 {
        return Err(SnapshotterError::ExtractionFailed(format!(
            "invalid digest format: {}",
            digest
        )));
    }

    Ok(ross_store::Digest {
        algorithm: parts[0].to_string(),
        hash: parts[1].to_string(),
    })
}

fn extract_tar_gz(data: &[u8], target_dir: &Path) -> Result<i64, SnapshotterError> {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);
    archive.set_overwrite(true);

    // On macOS, we can't preserve Linux-specific permissions/ownerships
    #[cfg(not(target_os = "macos"))]
    {
        archive.set_preserve_permissions(true);
        archive.set_preserve_ownerships(true);
        archive.set_unpack_xattrs(true);
    }

    let mut total_size = 0i64;

    for entry in archive.entries().map_err(|e| {
        SnapshotterError::ExtractionFailed(format!("failed to read tar entries: {}", e))
    })? {
        let mut entry = entry.map_err(|e| {
            SnapshotterError::ExtractionFailed(format!("failed to read tar entry: {}", e))
        })?;

        let path = entry.path().map_err(|e| {
            SnapshotterError::ExtractionFailed(format!("failed to get entry path: {}", e))
        })?.into_owned();

        // Handle whiteout files (OCI layer deletion markers)
        if let Some(name) = path.file_name() {
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".wh.") {
                let original_name = name_str.strip_prefix(".wh.").unwrap();
                let whiteout_target = target_dir
                    .join(path.parent().unwrap_or(Path::new("")))
                    .join(original_name);
                if whiteout_target.exists() {
                    if whiteout_target.is_dir() {
                        std::fs::remove_dir_all(&whiteout_target).map_err(|e| {
                            SnapshotterError::ExtractionFailed(format!(
                                "failed to remove whiteout target: {}",
                                e
                            ))
                        })?;
                    } else {
                        std::fs::remove_file(&whiteout_target).map_err(|e| {
                            SnapshotterError::ExtractionFailed(format!(
                                "failed to remove whiteout target: {}",
                                e
                            ))
                        })?;
                    }
                }
                continue;
            }
        }

        // Skip device nodes on macOS (can't create them without root)
        #[cfg(target_os = "macos")]
        {
            let entry_type = entry.header().entry_type();
            if entry_type == tar::EntryType::Char || entry_type == tar::EntryType::Block {
                tracing::debug!("Skipping device node: {:?}", path);
                continue;
            }
        }

        total_size += entry.size() as i64;
        
        // Try to unpack, but on macOS handle failures gracefully for special files
        #[cfg(target_os = "macos")]
        {
            let entry_type = entry.header().entry_type();
            if let Err(e) = entry.unpack_in(target_dir) {
                // Only error for regular files/dirs, skip special files
                if entry_type == tar::EntryType::Regular 
                    || entry_type == tar::EntryType::Directory
                    || entry_type == tar::EntryType::Symlink
                    || entry_type == tar::EntryType::Link
                {
                    return Err(SnapshotterError::ExtractionFailed(format!(
                        "failed to unpack {:?}: {}",
                        path, e
                    )));
                }
                tracing::debug!("Skipping special file {:?}: {}", path, e);
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            entry.unpack_in(target_dir).map_err(|e| {
                SnapshotterError::ExtractionFailed(format!("failed to unpack entry: {}", e))
            })?;
        }
    }

    Ok(total_size)
}

async fn calculate_dir_usage(dir: &Path) -> Result<(i64, i64), SnapshotterError> {
    let mut size = 0i64;
    let mut inodes = 0i64;

    if !dir.exists() {
        return Ok((0, 0));
    }

    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let mut entries = fs::read_dir(&current).await?;
        while let Some(entry) = entries.next_entry().await? {
            inodes += 1;
            let metadata = entry.metadata().await?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                size += metadata.len() as i64;
            }
        }
    }

    Ok((size, inodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_snapshotter() -> (OverlaySnapshotter, TempDir, TempDir) {
        let snap_dir = TempDir::new().unwrap();
        let store_dir = TempDir::new().unwrap();
        let store = Arc::new(FileSystemStore::new(store_dir.path()).await.unwrap());
        let snapshotter = OverlaySnapshotter::new(snap_dir.path(), store)
            .await
            .unwrap();
        (snapshotter, snap_dir, store_dir)
    }

    #[tokio::test]
    async fn test_prepare_and_commit() {
        let (snapshotter, _snap_dir, _store_dir) = create_test_snapshotter().await;

        let mounts = snapshotter
            .prepare("test-active", None, HashMap::new())
            .await
            .unwrap();

        assert!(!mounts.is_empty());

        let info = snapshotter.stat("test-active").await.unwrap();
        assert_eq!(info.kind, SnapshotKind::Active);

        snapshotter
            .commit("test-committed", "test-active", HashMap::new())
            .await
            .unwrap();

        let info = snapshotter.stat("test-committed").await.unwrap();
        assert_eq!(info.kind, SnapshotKind::Committed);

        assert!(snapshotter.stat("test-active").await.is_err());
    }

    #[tokio::test]
    async fn test_parent_chain() {
        let (snapshotter, _snap_dir, _store_dir) = create_test_snapshotter().await;

        snapshotter
            .prepare("layer1-active", None, HashMap::new())
            .await
            .unwrap();
        snapshotter
            .commit("layer1", "layer1-active", HashMap::new())
            .await
            .unwrap();

        snapshotter
            .prepare("layer2-active", Some("layer1"), HashMap::new())
            .await
            .unwrap();
        snapshotter
            .commit("layer2", "layer2-active", HashMap::new())
            .await
            .unwrap();

        let mounts = snapshotter
            .prepare("container", Some("layer2"), HashMap::new())
            .await
            .unwrap();

        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].mount_type, "overlay");

        let options = &mounts[0].options;
        let lowerdir = options.iter().find(|o| o.starts_with("lowerdir=")).unwrap();
        assert!(lowerdir.contains("layer2"));
        assert!(lowerdir.contains("layer1"));
    }

    #[tokio::test]
    async fn test_remove_with_dependents() {
        let (snapshotter, _snap_dir, _store_dir) = create_test_snapshotter().await;

        snapshotter
            .prepare("parent-active", None, HashMap::new())
            .await
            .unwrap();
        snapshotter
            .commit("parent", "parent-active", HashMap::new())
            .await
            .unwrap();

        snapshotter
            .prepare("child", Some("parent"), HashMap::new())
            .await
            .unwrap();

        let result = snapshotter.remove("parent").await;
        assert!(matches!(result, Err(SnapshotterError::HasDependents(_))));
    }

    #[tokio::test]
    async fn test_view() {
        let (snapshotter, _snap_dir, _store_dir) = create_test_snapshotter().await;

        snapshotter
            .prepare("base-active", None, HashMap::new())
            .await
            .unwrap();
        snapshotter
            .commit("base", "base-active", HashMap::new())
            .await
            .unwrap();

        let mounts = snapshotter
            .view("readonly-view", Some("base"), HashMap::new())
            .await
            .unwrap();

        let info = snapshotter.stat("readonly-view").await.unwrap();
        assert_eq!(info.kind, SnapshotKind::View);

        assert!(!mounts.is_empty());
    }
}
