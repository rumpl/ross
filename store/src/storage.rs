use crate::error::StoreError;
use crate::{BlobInfo, Digest, ManifestInfo, TagInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BLOBS_DIR: &str = "blobs";
const MANIFESTS_DIR: &str = "manifests";
const INDEXES_DIR: &str = "indexes";
const TAGS_DIR: &str = "tags";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMetadata {
    pub media_type: String,
    pub size: i64,
    pub created_at: i64,
    pub accessed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMetadata {
    pub media_type: String,
    pub size: i64,
    pub created_at: i64,
    pub schema_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagMetadata {
    pub digest_algorithm: String,
    pub digest_hash: String,
    pub updated_at: i64,
}

pub struct FileSystemStore {
    root: PathBuf,
}

impl FileSystemStore {
    pub async fn new(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();

        fs::create_dir_all(root.join(BLOBS_DIR)).await?;
        fs::create_dir_all(root.join(MANIFESTS_DIR)).await?;
        fs::create_dir_all(root.join(INDEXES_DIR)).await?;
        fs::create_dir_all(root.join(TAGS_DIR)).await?;

        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn blob_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(BLOBS_DIR)
            .join(&digest.algorithm)
            .join(&digest.hash)
    }

    fn blob_meta_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(BLOBS_DIR)
            .join(&digest.algorithm)
            .join(format!("{}.meta", digest.hash))
    }

    fn manifest_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(MANIFESTS_DIR)
            .join(&digest.algorithm)
            .join(&digest.hash)
    }

    fn manifest_meta_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(MANIFESTS_DIR)
            .join(&digest.algorithm)
            .join(format!("{}.meta", digest.hash))
    }

    fn index_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(INDEXES_DIR)
            .join(&digest.algorithm)
            .join(&digest.hash)
    }

    fn tag_path(&self, repository: &str, tag: &str) -> PathBuf {
        self.root.join(TAGS_DIR).join(repository).join(tag)
    }

    pub async fn has_blob(&self, digest: &Digest) -> bool {
        self.blob_path(digest).exists()
    }

    pub async fn get_blob(
        &self,
        digest: &Digest,
        offset: i64,
        length: i64,
    ) -> Result<Vec<u8>, StoreError> {
        let path = self.blob_path(digest);
        if !path.exists() {
            return Err(StoreError::BlobNotFound(format_digest(digest)));
        }

        let mut file = fs::File::open(&path).await?;
        let metadata = file.metadata().await?;
        let file_size = metadata.len() as i64;

        if offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(offset as u64)).await?;
        }

        let read_len = if length <= 0 {
            (file_size - offset) as usize
        } else {
            length as usize
        };

        let mut buf = vec![0u8; read_len];
        file.read_exact(&mut buf).await?;

        Ok(buf)
    }

    pub async fn put_blob(
        &self,
        media_type: &str,
        data: &[u8],
        expected_digest: Option<&Digest>,
    ) -> Result<(Digest, i64), StoreError> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hex::encode(hasher.finalize());

        let digest = Digest {
            algorithm: "sha256".to_string(),
            hash,
        };

        if let Some(expected) = expected_digest
            && expected.algorithm == digest.algorithm
            && expected.hash != digest.hash
        {
            return Err(StoreError::DigestMismatch {
                expected: format_digest(expected),
                actual: format_digest(&digest),
            });
        }

        let blob_path = self.blob_path(&digest);
        if let Some(parent) = blob_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::File::create(&blob_path).await?;
        file.write_all(data).await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let meta = BlobMetadata {
            media_type: media_type.to_string(),
            size: data.len() as i64,
            created_at: now,
            accessed_at: now,
        };

        let meta_path = self.blob_meta_path(&digest);
        let meta_json = serde_json::to_string(&meta)?;
        fs::write(&meta_path, meta_json).await?;

        Ok((digest, data.len() as i64))
    }

    pub async fn stat_blob(&self, digest: &Digest) -> Result<Option<BlobInfo>, StoreError> {
        let path = self.blob_path(digest);
        if !path.exists() {
            return Ok(None);
        }

        let meta_path = self.blob_meta_path(digest);
        let meta: BlobMetadata = if meta_path.exists() {
            let content = fs::read_to_string(&meta_path).await?;
            serde_json::from_str(&content)?
        } else {
            let metadata = fs::metadata(&path).await?;
            BlobMetadata {
                media_type: "application/octet-stream".to_string(),
                size: metadata.len() as i64,
                created_at: 0,
                accessed_at: 0,
            }
        };

        Ok(Some(BlobInfo {
            digest: Some(digest.clone()),
            size: meta.size,
            media_type: meta.media_type,
            created_at: Some(prost_types::Timestamp {
                seconds: meta.created_at,
                nanos: 0,
            }),
            accessed_at: Some(prost_types::Timestamp {
                seconds: meta.accessed_at,
                nanos: 0,
            }),
        }))
    }

    pub async fn delete_blob(&self, digest: &Digest) -> Result<bool, StoreError> {
        let path = self.blob_path(digest);
        if !path.exists() {
            return Ok(false);
        }

        fs::remove_file(&path).await?;

        let meta_path = self.blob_meta_path(digest);
        if meta_path.exists() {
            let _ = fs::remove_file(&meta_path).await;
        }

        Ok(true)
    }

    pub async fn list_blobs(
        &self,
        media_type_filter: Option<&str>,
    ) -> Result<Vec<BlobInfo>, StoreError> {
        let blobs_dir = self.root.join(BLOBS_DIR);
        let mut blobs = Vec::new();

        let mut algo_entries = fs::read_dir(&blobs_dir).await?;
        while let Some(algo_entry) = algo_entries.next_entry().await? {
            if !algo_entry.file_type().await?.is_dir() {
                continue;
            }
            let algorithm = algo_entry.file_name().to_string_lossy().to_string();

            let mut hash_entries = fs::read_dir(algo_entry.path()).await?;
            while let Some(hash_entry) = hash_entries.next_entry().await? {
                let name = hash_entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".meta") {
                    continue;
                }

                let digest = Digest {
                    algorithm: algorithm.clone(),
                    hash: name,
                };

                if let Some(info) = self.stat_blob(&digest).await? {
                    if let Some(filter) = media_type_filter
                        && !info.media_type.contains(filter)
                    {
                        continue;
                    }
                    blobs.push(info);
                }
            }
        }

        Ok(blobs)
    }

    pub async fn get_manifest(&self, digest: &Digest) -> Result<(Vec<u8>, String), StoreError> {
        let path = self.manifest_path(digest);
        if !path.exists() {
            return Err(StoreError::ManifestNotFound(format_digest(digest)));
        }

        let content = fs::read(&path).await?;

        let meta_path = self.manifest_meta_path(digest);
        let media_type = if meta_path.exists() {
            let meta_content = fs::read_to_string(&meta_path).await?;
            let meta: ManifestMetadata = serde_json::from_str(&meta_content)?;
            meta.media_type
        } else {
            "application/vnd.oci.image.manifest.v1+json".to_string()
        };

        Ok((content, media_type))
    }

    pub async fn put_manifest(
        &self,
        content: &[u8],
        media_type: &str,
    ) -> Result<(Digest, i64), StoreError> {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash = hex::encode(hasher.finalize());

        let digest = Digest {
            algorithm: "sha256".to_string(),
            hash,
        };

        let manifest_path = self.manifest_path(&digest);
        if let Some(parent) = manifest_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&manifest_path, content).await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let meta = ManifestMetadata {
            media_type: media_type.to_string(),
            size: content.len() as i64,
            created_at: now,
            schema_version: "2".to_string(),
        };

        let meta_path = self.manifest_meta_path(&digest);
        let meta_json = serde_json::to_string(&meta)?;
        fs::write(&meta_path, meta_json).await?;

        Ok((digest, content.len() as i64))
    }

    pub async fn delete_manifest(&self, digest: &Digest) -> Result<bool, StoreError> {
        let path = self.manifest_path(digest);
        if !path.exists() {
            return Ok(false);
        }

        fs::remove_file(&path).await?;

        let meta_path = self.manifest_meta_path(digest);
        if meta_path.exists() {
            let _ = fs::remove_file(&meta_path).await;
        }

        Ok(true)
    }

    pub async fn list_manifests(
        &self,
        media_type_filter: Option<&str>,
    ) -> Result<Vec<ManifestInfo>, StoreError> {
        let manifests_dir = self.root.join(MANIFESTS_DIR);
        let mut manifests = Vec::new();

        if !manifests_dir.exists() {
            return Ok(manifests);
        }

        let mut algo_entries = fs::read_dir(&manifests_dir).await?;
        while let Some(algo_entry) = algo_entries.next_entry().await? {
            if !algo_entry.file_type().await?.is_dir() {
                continue;
            }
            let algorithm = algo_entry.file_name().to_string_lossy().to_string();

            let mut hash_entries = fs::read_dir(algo_entry.path()).await?;
            while let Some(hash_entry) = hash_entries.next_entry().await? {
                let name = hash_entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".meta") {
                    continue;
                }

                let digest = Digest {
                    algorithm: algorithm.clone(),
                    hash: name,
                };

                let meta_path = self.manifest_meta_path(&digest);
                let meta: ManifestMetadata = if meta_path.exists() {
                    let content = fs::read_to_string(&meta_path).await?;
                    serde_json::from_str(&content)?
                } else {
                    let path = self.manifest_path(&digest);
                    let metadata = fs::metadata(&path).await?;
                    ManifestMetadata {
                        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
                        size: metadata.len() as i64,
                        created_at: 0,
                        schema_version: "2".to_string(),
                    }
                };

                if let Some(filter) = media_type_filter
                    && !meta.media_type.contains(filter)
                {
                    continue;
                }

                manifests.push(ManifestInfo {
                    digest: Some(digest),
                    size: meta.size,
                    media_type: meta.media_type,
                    created_at: Some(prost_types::Timestamp {
                        seconds: meta.created_at,
                        nanos: 0,
                    }),
                    schema_version: meta.schema_version,
                });
            }
        }

        Ok(manifests)
    }

    pub async fn get_index(&self, digest: &Digest) -> Result<Vec<u8>, StoreError> {
        let path = self.index_path(digest);
        if !path.exists() {
            return Err(StoreError::ManifestNotFound(format_digest(digest)));
        }
        Ok(fs::read(&path).await?)
    }

    pub async fn put_index(&self, content: &[u8]) -> Result<(Digest, i64), StoreError> {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash = hex::encode(hasher.finalize());

        let digest = Digest {
            algorithm: "sha256".to_string(),
            hash,
        };

        let index_path = self.index_path(&digest);
        if let Some(parent) = index_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&index_path, content).await?;
        Ok((digest, content.len() as i64))
    }

    pub async fn delete_index(&self, digest: &Digest) -> Result<bool, StoreError> {
        let path = self.index_path(digest);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path).await?;
        Ok(true)
    }

    pub async fn resolve_tag(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<(Digest, String), StoreError> {
        let path = self.tag_path(repository, tag);
        if !path.exists() {
            return Err(StoreError::TagNotFound(
                repository.to_string(),
                tag.to_string(),
            ));
        }

        let content = fs::read_to_string(&path).await?;
        let meta: TagMetadata = serde_json::from_str(&content)?;

        let digest = Digest {
            algorithm: meta.digest_algorithm,
            hash: meta.digest_hash,
        };

        let manifest_path = self.manifest_path(&digest);
        let media_type = if manifest_path.exists() {
            let meta_path = self.manifest_meta_path(&digest);
            if meta_path.exists() {
                let m: ManifestMetadata =
                    serde_json::from_str(&fs::read_to_string(&meta_path).await?)?;
                m.media_type
            } else {
                "application/vnd.oci.image.manifest.v1+json".to_string()
            }
        } else {
            "application/vnd.oci.image.index.v1+json".to_string()
        };

        Ok((digest, media_type))
    }

    pub async fn set_tag(
        &self,
        repository: &str,
        tag: &str,
        digest: &Digest,
    ) -> Result<Option<Digest>, StoreError> {
        let path = self.tag_path(repository, tag);

        let previous = if path.exists() {
            let content = fs::read_to_string(&path).await?;
            let meta: TagMetadata = serde_json::from_str(&content)?;
            Some(Digest {
                algorithm: meta.digest_algorithm,
                hash: meta.digest_hash,
            })
        } else {
            None
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let meta = TagMetadata {
            digest_algorithm: digest.algorithm.clone(),
            digest_hash: digest.hash.clone(),
            updated_at: now,
        };

        let meta_json = serde_json::to_string(&meta)?;
        fs::write(&path, meta_json).await?;

        Ok(previous)
    }

    pub async fn delete_tag(&self, repository: &str, tag: &str) -> Result<bool, StoreError> {
        let path = self.tag_path(repository, tag);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path).await?;
        Ok(true)
    }

    pub async fn list_tags(&self, repository: &str) -> Result<Vec<TagInfo>, StoreError> {
        let repo_dir = self.root.join(TAGS_DIR).join(repository);
        let mut tags = Vec::new();

        if !repo_dir.exists() {
            return Ok(tags);
        }

        let mut entries = fs::read_dir(&repo_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }

            let tag_name = entry.file_name().to_string_lossy().to_string();
            let content = fs::read_to_string(entry.path()).await?;
            let meta: TagMetadata = serde_json::from_str(&content)?;

            tags.push(TagInfo {
                tag: tag_name,
                digest: Some(Digest {
                    algorithm: meta.digest_algorithm,
                    hash: meta.digest_hash,
                }),
                updated_at: Some(prost_types::Timestamp {
                    seconds: meta.updated_at,
                    nanos: 0,
                }),
            });
        }

        Ok(tags)
    }

    pub async fn garbage_collect(
        &self,
        dry_run: bool,
        delete_untagged: bool,
    ) -> Result<(i64, i64, i64, Vec<Digest>), StoreError> {
        let mut referenced_digests: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let tags_dir = self.root.join(TAGS_DIR);
        if tags_dir.exists() {
            let mut repo_entries = fs::read_dir(&tags_dir).await?;
            while let Some(repo_entry) = repo_entries.next_entry().await? {
                if !repo_entry.file_type().await?.is_dir() {
                    continue;
                }
                let mut tag_entries = fs::read_dir(repo_entry.path()).await?;
                while let Some(tag_entry) = tag_entries.next_entry().await? {
                    if let Ok(content) = fs::read_to_string(tag_entry.path()).await
                        && let Ok(meta) = serde_json::from_str::<TagMetadata>(&content)
                    {
                        referenced_digests
                            .insert(format!("{}:{}", meta.digest_algorithm, meta.digest_hash));
                    }
                }
            }
        }

        let mut removed_digests = Vec::new();
        let blobs_removed = 0i64;
        let mut manifests_removed = 0i64;
        let mut bytes_freed = 0i64;

        if delete_untagged {
            for manifest in self.list_manifests(None).await? {
                if let Some(digest) = &manifest.digest {
                    let key = format!("{}:{}", digest.algorithm, digest.hash);
                    if !referenced_digests.contains(&key) {
                        if !dry_run {
                            self.delete_manifest(digest).await?;
                        }
                        bytes_freed += manifest.size;
                        manifests_removed += 1;
                        removed_digests.push(digest.clone());
                    }
                }
            }
        }

        Ok((blobs_removed, manifests_removed, bytes_freed, removed_digests))
    }

    pub async fn get_store_info(&self) -> Result<(i64, i64, i64, i64), StoreError> {
        let blobs = self.list_blobs(None).await?;
        let manifests = self.list_manifests(None).await?;

        let total_size: i64 =
            blobs.iter().map(|b| b.size).sum::<i64>() + manifests.iter().map(|m| m.size).sum::<i64>();

        let mut tag_count = 0i64;
        let tags_dir = self.root.join(TAGS_DIR);
        if tags_dir.exists() {
            let mut repo_entries = fs::read_dir(&tags_dir).await?;
            while let Some(repo_entry) = repo_entries.next_entry().await? {
                if repo_entry.file_type().await?.is_dir() {
                    let mut tag_entries = fs::read_dir(repo_entry.path()).await?;
                    while (tag_entries.next_entry().await?).is_some() {
                        tag_count += 1;
                    }
                }
            }
        }

        Ok((total_size, blobs.len() as i64, manifests.len() as i64, tag_count))
    }
}

fn format_digest(digest: &Digest) -> String {
    format!("{}:{}", digest.algorithm, digest.hash)
}
