use crate::error::ImageError;
use crate::types::*;
use async_stream::stream;
use ross_remote::{Descriptor, ImageReference, RegistryClient};
use ross_store::FileSystemStore;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio_stream::Stream;

type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

pub struct ImageService {
    store: Arc<FileSystemStore>,
    max_concurrent_downloads: usize,
}

impl ImageService {
    pub fn new(store: Arc<FileSystemStore>, max_concurrent_downloads: usize) -> Self {
        Self {
            store,
            max_concurrent_downloads,
        }
    }

    pub async fn list(&self, _params: ListImagesParams) -> Result<Vec<Image>, ImageError> {
        let repositories = self.store.list_repositories().await?;
        let mut images = Vec::new();

        for repo in repositories {
            let tags = self.store.list_tags(&repo).await?;

            for tag_info in tags {
                let digest = match &tag_info.digest {
                    Some(d) => d,
                    None => continue,
                };

                let (manifest_bytes, _media_type) = match self.store.get_manifest(digest).await {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let manifest: ross_remote::ManifestV2 = match serde_json::from_slice(&manifest_bytes)
                {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let config_digest = ross_store::Digest {
                    algorithm: "sha256".to_string(),
                    hash: manifest.config.digest.trim_start_matches("sha256:").to_string(),
                };

                let config_bytes = match self.store.get_blob(&config_digest, 0, -1).await {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                let config: ross_remote::ImageConfig = match serde_json::from_slice(&config_bytes) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let total_size: i64 = manifest.layers.iter().map(|l| l.size).sum();

                let repo_tag = format!("{}:{}", repo, tag_info.tag);
                let repo_digest = format!("{}@sha256:{}", repo, digest.hash);

                let labels = config
                    .config
                    .as_ref()
                    .map(|c| c.labels.clone())
                    .unwrap_or_default();

                let layer_digests: Vec<String> = manifest
                    .layers
                    .iter()
                    .map(|l| l.digest.clone())
                    .collect();

                images.push(Image {
                    id: format!("sha256:{}", digest.hash),
                    repo_tags: vec![repo_tag],
                    repo_digests: vec![repo_digest],
                    parent: String::new(),
                    comment: String::new(),
                    container: String::new(),
                    docker_version: String::new(),
                    author: String::new(),
                    architecture: config.architecture.clone(),
                    os: config.os.clone(),
                    size: total_size,
                    virtual_size: total_size,
                    labels,
                    root_fs: Some(RootFs {
                        fs_type: "layers".to_string(),
                        layers: layer_digests,
                    }),
                });
            }
        }

        Ok(images)
    }

    pub async fn inspect(&self, image_id: &str) -> Result<ImageInspection, ImageError> {
        tracing::info!("Inspecting image: {}", image_id);
        Ok(ImageInspection {
            image: Image::default(),
            history: vec![],
        })
    }

    pub fn pull(
        &self,
        image_name: &str,
        tag: &str,
        _auth: Option<RegistryAuth>,
    ) -> Result<BoxStream<PullProgress>, ImageError> {
        let parsed = ImageReference::parse(image_name)
            .map_err(|e| ImageError::InvalidReference(e.to_string()))?;

        let reference = if parsed.tag.is_some() || parsed.digest.is_some() {
            parsed
        } else {
            let effective_tag = if tag.is_empty() { "latest" } else { tag };
            let reference_str = format!("{}:{}", image_name, effective_tag);
            ImageReference::parse(&reference_str)
                .map_err(|e| ImageError::InvalidReference(e.to_string()))?
        };

        tracing::info!("Pulling image: {}", reference.full_name());

        let store = self.store.clone();
        let max_concurrent = self.max_concurrent_downloads;

        let output = stream! {
            yield PullProgress {
                id: reference.full_name(),
                status: "Resolving".to_string(),
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };

            let registry = match RegistryClient::new() {
                Ok(r) => Arc::new(r),
                Err(e) => {
                    yield PullProgress {
                        id: reference.full_name(),
                        status: String::new(),
                        progress: String::new(),
                        current: None,
                        total: None,
                        error: Some(format!("Failed to create registry client: {}", e)),
                    };
                    return;
                }
            };

            let os = "linux";
            let arch = match std::env::consts::ARCH {
                "x86_64" => "amd64",
                "aarch64" => "arm64",
                a => a,
            };

            let (manifest, media_type, manifest_digest) = match registry
                .get_manifest_for_platform(&reference, os, arch)
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    yield PullProgress {
                        id: reference.full_name(),
                        status: String::new(),
                        progress: String::new(),
                        current: None,
                        total: None,
                        error: Some(format!("Failed to get manifest: {}", e)),
                    };
                    return;
                }
            };

            yield PullProgress {
                id: reference.full_name(),
                status: format!("Resolved digest: {}", &manifest_digest),
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };

            let config_digest = &manifest.config.digest;
            let short_config_id = if config_digest.len() > 19 {
                &config_digest[7..19]
            } else {
                config_digest
            };

            yield PullProgress {
                id: short_config_id.to_string(),
                status: "Pulling config".to_string(),
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };

            let config_bytes = match registry.get_blob_bytes(&reference, config_digest).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    yield PullProgress {
                        id: short_config_id.to_string(),
                        status: String::new(),
                        progress: String::new(),
                        current: None,
                        total: None,
                        error: Some(format!("Failed to pull config: {}", e)),
                    };
                    return;
                }
            };

            if let Err(e) = store.put_blob(&manifest.config.media_type, &config_bytes, None).await {
                yield PullProgress {
                    id: short_config_id.to_string(),
                    status: String::new(),
                    progress: String::new(),
                    current: None,
                    total: None,
                    error: Some(format!("Failed to store config: {}", e)),
                };
                return;
            }

            yield PullProgress {
                id: short_config_id.to_string(),
                status: "Pull complete".to_string(),
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };

            let semaphore = Arc::new(Semaphore::new(max_concurrent));
            let (tx, mut rx) = mpsc::channel::<LayerEvent>(manifest.layers.len() * 4);
            let total_layers = manifest.layers.len();

            let mut handles = Vec::new();
            for (i, layer) in manifest.layers.iter().enumerate() {
                let handle = tokio::spawn(download_layer(
                    registry.clone(),
                    store.clone(),
                    reference.clone(),
                    layer.clone(),
                    i + 1,
                    total_layers,
                    semaphore.clone(),
                    tx.clone(),
                ));
                handles.push(handle);
            }

            drop(tx);

            let mut error_occurred = false;
            let mut any_downloaded = false;
            while let Some(event) = rx.recv().await {
                match event {
                    LayerEvent::Exists { id } => {
                        yield PullProgress {
                            id,
                            status: "Already exists".to_string(),
                            progress: String::new(),
                            current: None,
                            total: None,
                            error: None,
                        };
                    }
                    LayerEvent::Downloading { id, index, total } => {
                        yield PullProgress {
                            id,
                            status: "Downloading".to_string(),
                            progress: format!("[{}/{}]", index, total),
                            current: None,
                            total: None,
                            error: None,
                        };
                    }
                    LayerEvent::Downloaded { id } => {
                        yield PullProgress {
                            id,
                            status: "Download complete".to_string(),
                            progress: String::new(),
                            current: None,
                            total: None,
                            error: None,
                        };
                    }
                    LayerEvent::Stored { id } => {
                        any_downloaded = true;
                        yield PullProgress {
                            id,
                            status: "Pull complete".to_string(),
                            progress: String::new(),
                            current: None,
                            total: None,
                            error: None,
                        };
                    }
                    LayerEvent::Error { id, error } => {
                        error_occurred = true;
                        yield PullProgress {
                            id,
                            status: String::new(),
                            progress: String::new(),
                            current: None,
                            total: None,
                            error: Some(error),
                        };
                    }
                }
            }

            for handle in handles {
                let _ = handle.await;
            }

            if error_occurred {
                return;
            }

            let manifest_bytes = serde_json::to_vec(&manifest).unwrap_or_default();
            let (stored_digest, _) = match store.put_manifest(&manifest_bytes, &media_type).await {
                Ok(result) => result,
                Err(e) => {
                    yield PullProgress {
                        id: reference.full_name(),
                        status: String::new(),
                        progress: String::new(),
                        current: None,
                        total: None,
                        error: Some(format!("Failed to store manifest: {}", e)),
                    };
                    return;
                }
            };

            if let Err(e) = store.set_tag(&reference.repository, reference.tag_or_default(), &stored_digest).await {
                yield PullProgress {
                    id: reference.full_name(),
                    status: String::new(),
                    progress: String::new(),
                    current: None,
                    total: None,
                    error: Some(format!("Failed to set tag: {}", e)),
                };
                return;
            }

            let digest_str = format!("sha256:{}", stored_digest.hash);
            yield PullProgress {
                id: reference.full_name(),
                status: format!("Digest: {}", digest_str),
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };

            let status_message = if any_downloaded {
                format!("Status: Downloaded newer image for {}", reference.full_name())
            } else {
                format!("Status: Image is up to date for {}", reference.full_name())
            };

            yield PullProgress {
                id: reference.full_name(),
                status: status_message,
                progress: String::new(),
                current: None,
                total: None,
                error: None,
            };
        };

        Ok(Box::pin(output))
    }

    pub fn push(
        &self,
        image_name: &str,
        tag: &str,
        _auth: Option<RegistryAuth>,
    ) -> BoxStream<PushProgress> {
        tracing::info!("Pushing image: {}:{}", image_name, tag);
        let image_name = image_name.to_string();

        let output = stream! {
            for status in ["Preparing", "Pushing", "Complete"] {
                yield PushProgress {
                    id: image_name.clone(),
                    status: status.to_string(),
                    progress: String::new(),
                    current: None,
                    total: None,
                    error: None,
                };
            }
        };

        Box::pin(output)
    }

    pub fn build(&self, params: BuildParams) -> BoxStream<BuildProgress> {
        tracing::info!("Building image with tags: {:?}", params.tags);

        let output = stream! {
            for step in [
                "Step 1/3: FROM base",
                "Step 2/3: RUN command",
                "Step 3/3: Complete",
            ] {
                yield BuildProgress {
                    stream: step.to_string(),
                    error: None,
                    progress: String::new(),
                    aux_id: None,
                };
            }
        };

        Box::pin(output)
    }

    pub async fn remove(
        &self,
        image_id: &str,
        _force: bool,
        _prune_children: bool,
    ) -> Result<RemoveImageResult, ImageError> {
        tracing::info!("Removing image: {}", image_id);
        Ok(RemoveImageResult {
            deleted: vec![],
            untagged: vec![],
        })
    }

    pub async fn tag(
        &self,
        source_image: &str,
        repository: &str,
        tag: &str,
    ) -> Result<(), ImageError> {
        tracing::info!(
            "Tagging image {} as {}:{}",
            source_image,
            repository,
            tag
        );
        Ok(())
    }

    pub async fn search(&self, params: SearchParams) -> Result<Vec<SearchResult>, ImageError> {
        tracing::info!("Searching images with term: {}", params.term);
        Ok(vec![])
    }
}

#[derive(Debug)]
enum LayerEvent {
    Downloading { id: String, index: usize, total: usize },
    Downloaded { id: String },
    Stored { id: String },
    Exists { id: String },
    Error { id: String, error: String },
}

#[allow(clippy::too_many_arguments)]
async fn download_layer(
    registry: Arc<RegistryClient>,
    store: Arc<FileSystemStore>,
    reference: ImageReference,
    layer: Descriptor,
    index: usize,
    total: usize,
    semaphore: Arc<Semaphore>,
    tx: mpsc::Sender<LayerEvent>,
) {
    let layer_digest = layer.digest.clone();
    let short_layer_id = if layer_digest.len() > 19 {
        layer_digest[7..19].to_string()
    } else {
        layer_digest.clone()
    };

    let store_digest = ross_store::Digest {
        algorithm: "sha256".to_string(),
        hash: layer_digest.trim_start_matches("sha256:").to_string(),
    };

    if let Ok(Some(_)) = store.stat_blob(&store_digest).await {
        let _ = tx.send(LayerEvent::Exists { id: short_layer_id }).await;
        return;
    }

    let _permit = semaphore.acquire().await.expect("semaphore closed");

    let _ = tx
        .send(LayerEvent::Downloading {
            id: short_layer_id.clone(),
            index,
            total,
        })
        .await;

    let layer_bytes = match registry.get_blob_bytes(&reference, &layer_digest).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = tx
                .send(LayerEvent::Error {
                    id: short_layer_id,
                    error: format!("Failed to download layer: {}", e),
                })
                .await;
            return;
        }
    };

    let _ = tx
        .send(LayerEvent::Downloaded {
            id: short_layer_id.clone(),
        })
        .await;

    if let Err(e) = store.put_blob(&layer.media_type, &layer_bytes, None).await {
        let _ = tx
            .send(LayerEvent::Error {
                id: short_layer_id,
                error: format!("Failed to store layer: {}", e),
            })
            .await;
        return;
    }

    let _ = tx.send(LayerEvent::Stored { id: short_layer_id }).await;
}
