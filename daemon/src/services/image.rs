use async_stream::stream;
use ross_core::image_service_server::ImageService;
use ross_core::{
    BuildImageProgress, BuildImageRequest, Image, InspectImageRequest, InspectImageResponse,
    ListImagesRequest, ListImagesResponse, PullImageProgress, PullImageRequest, PushImageProgress,
    PushImageRequest, RemoveImageRequest, RemoveImageResponse, SearchImagesRequest,
    SearchImagesResponse, TagImageRequest, TagImageResponse,
};
use ross_remote::{ImageReference, RegistryClient};
use ross_store::FileSystemStore;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

#[allow(dead_code)]
pub struct ImageServiceImpl {
    store: Arc<FileSystemStore>,
    registry: RegistryClient,
}

impl ImageServiceImpl {
    pub fn new(store: Arc<FileSystemStore>) -> Self {
        Self {
            store,
            registry: RegistryClient::new().expect("failed to create registry client"),
        }
    }
}

#[tonic::async_trait]
impl ImageService for ImageServiceImpl {
    async fn list_images(
        &self,
        request: Request<ListImagesRequest>,
    ) -> Result<Response<ListImagesResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Listing images with filters: {:?}", req.filters);
        Ok(Response::new(ListImagesResponse { images: vec![] }))
    }

    async fn inspect_image(
        &self,
        request: Request<InspectImageRequest>,
    ) -> Result<Response<InspectImageResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Inspecting image: {}", req.image_id);
        Ok(Response::new(InspectImageResponse {
            image: Some(Image::default()),
            history: vec![],
        }))
    }

    type PullImageStream = StreamResult<PullImageProgress>;

    async fn pull_image(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<Self::PullImageStream>, Status> {
        let req = request.into_inner();
        let image_name = req.image_name.clone();
        let tag = if req.tag.is_empty() {
            "latest".to_string()
        } else {
            req.tag.clone()
        };

        let reference_str = format!("{}:{}", image_name, tag);
        tracing::info!("Pulling image: {}", reference_str);

        let reference = ImageReference::parse(&reference_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let store = self.store.clone();
        let registry = RegistryClient::new()
            .map_err(|e| Status::internal(e.to_string()))?;

        let output = stream! {
            yield Ok(PullImageProgress {
                id: reference.full_name(),
                status: "Resolving".to_string(),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });

            // Container images are built for linux, not the host OS
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
                    yield Ok(PullImageProgress {
                        id: reference.full_name(),
                        status: String::new(),
                        progress: String::new(),
                        progress_detail: None,
                        error: format!("Failed to get manifest: {}", e),
                    });
                    return;
                }
            };

            yield Ok(PullImageProgress {
                id: reference.full_name(),
                status: format!("Resolved digest: {}", &manifest_digest),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });

            let config_digest = &manifest.config.digest;
            let short_config_id = if config_digest.len() > 19 {
                &config_digest[7..19]
            } else {
                config_digest
            };

            yield Ok(PullImageProgress {
                id: short_config_id.to_string(),
                status: "Pulling config".to_string(),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });

            let config_bytes = match registry.get_blob_bytes(&reference, config_digest).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    yield Ok(PullImageProgress {
                        id: short_config_id.to_string(),
                        status: String::new(),
                        progress: String::new(),
                        progress_detail: None,
                        error: format!("Failed to pull config: {}", e),
                    });
                    return;
                }
            };

            if let Err(e) = store.put_blob(&manifest.config.media_type, &config_bytes, None).await {
                yield Ok(PullImageProgress {
                    id: short_config_id.to_string(),
                    status: String::new(),
                    progress: String::new(),
                    progress_detail: None,
                    error: format!("Failed to store config: {}", e),
                });
                return;
            }

            yield Ok(PullImageProgress {
                id: short_config_id.to_string(),
                status: "Pull complete".to_string(),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });

            for (i, layer) in manifest.layers.iter().enumerate() {
                let layer_digest = &layer.digest;
                let short_layer_id = if layer_digest.len() > 19 {
                    &layer_digest[7..19]
                } else {
                    layer_digest
                };

                let store_digest = ross_store::Digest {
                    algorithm: "sha256".to_string(),
                    hash: layer_digest.trim_start_matches("sha256:").to_string(),
                };

                if let Ok(Some(_)) = store.stat_blob(&store_digest).await {
                    yield Ok(PullImageProgress {
                        id: short_layer_id.to_string(),
                        status: "Already exists".to_string(),
                        progress: String::new(),
                        progress_detail: None,
                        error: String::new(),
                    });
                    continue;
                }

                yield Ok(PullImageProgress {
                    id: short_layer_id.to_string(),
                    status: "Downloading".to_string(),
                    progress: format!("[{}/{}]", i + 1, manifest.layers.len()),
                    progress_detail: Some(ross_core::ProgressDetail {
                        current: 0,
                        total: layer.size,
                    }),
                    error: String::new(),
                });

                let layer_bytes = match registry.get_blob_bytes(&reference, layer_digest).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        yield Ok(PullImageProgress {
                            id: short_layer_id.to_string(),
                            status: String::new(),
                            progress: String::new(),
                            progress_detail: None,
                            error: format!("Failed to download layer: {}", e),
                        });
                        return;
                    }
                };

                yield Ok(PullImageProgress {
                    id: short_layer_id.to_string(),
                    status: "Download complete".to_string(),
                    progress: String::new(),
                    progress_detail: None,
                    error: String::new(),
                });

                if let Err(e) = store.put_blob(&layer.media_type, &layer_bytes, None).await {
                    yield Ok(PullImageProgress {
                        id: short_layer_id.to_string(),
                        status: String::new(),
                        progress: String::new(),
                        progress_detail: None,
                        error: format!("Failed to store layer: {}", e),
                    });
                    return;
                }

                yield Ok(PullImageProgress {
                    id: short_layer_id.to_string(),
                    status: "Pull complete".to_string(),
                    progress: String::new(),
                    progress_detail: None,
                    error: String::new(),
                });
            }

            let manifest_bytes = serde_json::to_vec(&manifest).unwrap_or_default();
            let (stored_digest, _) = match store.put_manifest(&manifest_bytes, &media_type).await {
                Ok(result) => result,
                Err(e) => {
                    yield Ok(PullImageProgress {
                        id: reference.full_name(),
                        status: String::new(),
                        progress: String::new(),
                        progress_detail: None,
                        error: format!("Failed to store manifest: {}", e),
                    });
                    return;
                }
            };

            if let Err(e) = store.set_tag(&reference.repository, reference.tag_or_default(), &stored_digest).await {
                yield Ok(PullImageProgress {
                    id: reference.full_name(),
                    status: String::new(),
                    progress: String::new(),
                    progress_detail: None,
                    error: format!("Failed to set tag: {}", e),
                });
                return;
            }

            let digest_str = format!("sha256:{}", stored_digest.hash);
            yield Ok(PullImageProgress {
                id: reference.full_name(),
                status: format!("Digest: {}", digest_str),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });

            yield Ok(PullImageProgress {
                id: reference.full_name(),
                status: format!("Status: Downloaded newer image for {}", reference.full_name()),
                progress: String::new(),
                progress_detail: None,
                error: String::new(),
            });
        };

        Ok(Response::new(Box::pin(output)))
    }

    type PushImageStream = StreamResult<PushImageProgress>;

    async fn push_image(
        &self,
        request: Request<PushImageRequest>,
    ) -> Result<Response<Self::PushImageStream>, Status> {
        let req = request.into_inner();
        tracing::info!("Pushing image: {}:{}", req.image_name, req.tag);

        let output = stream! {
            for status in ["Preparing", "Pushing", "Complete"] {
                yield Ok(PushImageProgress {
                    status: status.to_string(),
                    progress: String::new(),
                    progress_detail: None,
                    id: req.image_name.clone(),
                    error: String::new(),
                });
            }
        };

        Ok(Response::new(Box::pin(output)))
    }

    type BuildImageStream = StreamResult<BuildImageProgress>;

    async fn build_image(
        &self,
        request: Request<BuildImageRequest>,
    ) -> Result<Response<Self::BuildImageStream>, Status> {
        let req = request.into_inner();
        tracing::info!("Building image with tags: {:?}", req.tags);

        let output = stream! {
            for step in [
                "Step 1/3: FROM base",
                "Step 2/3: RUN command",
                "Step 3/3: Complete",
            ] {
                yield Ok(BuildImageProgress {
                    stream: step.to_string(),
                    error: String::new(),
                    progress: String::new(),
                    aux: None,
                });
            }
        };

        Ok(Response::new(Box::pin(output)))
    }

    async fn remove_image(
        &self,
        request: Request<RemoveImageRequest>,
    ) -> Result<Response<RemoveImageResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Removing image: {}", req.image_id);
        Ok(Response::new(RemoveImageResponse {
            deleted: vec![],
            untagged: vec![],
        }))
    }

    async fn tag_image(
        &self,
        request: Request<TagImageRequest>,
    ) -> Result<Response<TagImageResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Tagging image {} as {}:{}",
            req.source_image,
            req.repository,
            req.tag
        );
        Ok(Response::new(TagImageResponse { success: true }))
    }

    async fn search_images(
        &self,
        request: Request<SearchImagesRequest>,
    ) -> Result<Response<SearchImagesResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Searching images with term: {}", req.term);
        Ok(Response::new(SearchImagesResponse { results: vec![] }))
    }
}
