use ross_core::image_service_server::ImageService as GrpcImageService;
use ross_core::{
    BuildImageProgress, BuildImageRequest, InspectImageRequest, InspectImageResponse,
    ListImagesRequest, ListImagesResponse, PullImageProgress, PullImageRequest, PushImageProgress,
    PushImageRequest, RemoveImageRequest, RemoveImageResponse, SearchImagesRequest,
    SearchImagesResponse, TagImageRequest, TagImageResponse,
};
use ross_image::{BuildParams, ImageService, ListImagesParams, RegistryAuth, SearchParams};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub struct ImageServiceGrpc {
    service: Arc<ImageService>,
}

impl ImageServiceGrpc {
    pub fn new(service: Arc<ImageService>) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl GrpcImageService for ImageServiceGrpc {
    async fn list_images(
        &self,
        request: Request<ListImagesRequest>,
    ) -> Result<Response<ListImagesResponse>, Status> {
        let req = request.into_inner();

        let params = ListImagesParams {
            all: req.all,
            filters: req.filters,
            digests: req.digests,
        };

        let images = self.service.list(params).await.map_err(into_status)?;

        Ok(Response::new(ListImagesResponse {
            images: images.into_iter().map(image_to_grpc).collect(),
        }))
    }

    async fn inspect_image(
        &self,
        request: Request<InspectImageRequest>,
    ) -> Result<Response<InspectImageResponse>, Status> {
        let req = request.into_inner();

        if req.image_id.is_empty() {
            return Err(Status::invalid_argument("image_id is required"));
        }

        let inspection = self
            .service
            .inspect(&req.image_id)
            .await
            .map_err(into_status)?;

        Ok(Response::new(InspectImageResponse {
            image: Some(image_to_grpc(inspection.image)),
            history: inspection.history.into_iter().map(history_to_grpc).collect(),
        }))
    }

    type PullImageStream = StreamResult<PullImageProgress>;

    async fn pull_image(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<Self::PullImageStream>, Status> {
        let req = request.into_inner();

        if req.image_name.is_empty() {
            return Err(Status::invalid_argument("image_name is required"));
        }

        let auth = req.registry_auth.map(registry_auth_from_grpc);

        let stream = self
            .service
            .pull(&req.image_name, &req.tag, auth)
            .map_err(into_status)?;

        let output = stream.map(|progress| Ok(pull_progress_to_grpc(progress)));

        Ok(Response::new(Box::pin(output)))
    }

    type PushImageStream = StreamResult<PushImageProgress>;

    async fn push_image(
        &self,
        request: Request<PushImageRequest>,
    ) -> Result<Response<Self::PushImageStream>, Status> {
        let req = request.into_inner();

        if req.image_name.is_empty() {
            return Err(Status::invalid_argument("image_name is required"));
        }

        let auth = req.registry_auth.map(registry_auth_from_grpc);

        let stream = self.service.push(&req.image_name, &req.tag, auth);
        let output = stream.map(|progress| Ok(push_progress_to_grpc(progress)));

        Ok(Response::new(Box::pin(output)))
    }

    type BuildImageStream = StreamResult<BuildImageProgress>;

    async fn build_image(
        &self,
        request: Request<BuildImageRequest>,
    ) -> Result<Response<Self::BuildImageStream>, Status> {
        let req = request.into_inner();

        let params = BuildParams {
            dockerfile: req.dockerfile,
            context_path: req.context_path,
            tags: req.tags,
            build_args: req.build_args,
            no_cache: req.no_cache,
            pull: req.pull,
            target: req.target,
            labels: req.labels,
            platform: req.platform,
        };

        let stream = self.service.build(params);
        let output = stream.map(|progress| Ok(build_progress_to_grpc(progress)));

        Ok(Response::new(Box::pin(output)))
    }

    async fn remove_image(
        &self,
        request: Request<RemoveImageRequest>,
    ) -> Result<Response<RemoveImageResponse>, Status> {
        let req = request.into_inner();

        if req.image_id.is_empty() {
            return Err(Status::invalid_argument("image_id is required"));
        }

        let result = self
            .service
            .remove(&req.image_id, req.force, req.prune_children)
            .await
            .map_err(into_status)?;

        Ok(Response::new(RemoveImageResponse {
            deleted: result.deleted,
            untagged: result.untagged,
        }))
    }

    async fn tag_image(
        &self,
        request: Request<TagImageRequest>,
    ) -> Result<Response<TagImageResponse>, Status> {
        let req = request.into_inner();

        if req.source_image.is_empty() {
            return Err(Status::invalid_argument("source_image is required"));
        }

        if req.repository.is_empty() {
            return Err(Status::invalid_argument("repository is required"));
        }

        self.service
            .tag(&req.source_image, &req.repository, &req.tag)
            .await
            .map_err(into_status)?;

        Ok(Response::new(TagImageResponse { success: true }))
    }

    async fn search_images(
        &self,
        request: Request<SearchImagesRequest>,
    ) -> Result<Response<SearchImagesResponse>, Status> {
        let req = request.into_inner();

        if req.term.is_empty() {
            return Err(Status::invalid_argument("term is required"));
        }

        let params = SearchParams {
            term: req.term,
            limit: req.limit,
            filters: req.filters,
        };

        let results = self.service.search(params).await.map_err(into_status)?;

        Ok(Response::new(SearchImagesResponse {
            results: results.into_iter().map(search_result_to_grpc).collect(),
        }))
    }
}

fn into_status(e: ross_image::ImageError) -> Status {
    match e {
        ross_image::ImageError::NotFound(_) => Status::not_found(e.to_string()),
        ross_image::ImageError::InvalidReference(_) => Status::invalid_argument(e.to_string()),
        ross_image::ImageError::PullFailed(_)
        | ross_image::ImageError::PushFailed(_)
        | ross_image::ImageError::BuildFailed(_) => Status::internal(e.to_string()),
        ross_image::ImageError::Registry(_)
        | ross_image::ImageError::Store(_)
        | ross_image::ImageError::Serialization(_) => Status::internal(e.to_string()),
    }
}

fn registry_auth_from_grpc(a: ross_core::RegistryAuth) -> RegistryAuth {
    RegistryAuth {
        username: a.username,
        password: a.password,
        server_address: a.server_address,
        identity_token: a.identity_token,
    }
}

fn image_to_grpc(i: ross_image::Image) -> ross_core::Image {
    ross_core::Image {
        id: i.id,
        repo_tags: i.repo_tags,
        repo_digests: i.repo_digests,
        parent: i.parent,
        comment: i.comment,
        created: None,
        container: i.container,
        docker_version: i.docker_version,
        author: i.author,
        architecture: i.architecture,
        os: i.os,
        size: i.size,
        virtual_size: i.virtual_size,
        labels: i.labels,
        root_fs: i.root_fs.map(root_fs_to_grpc),
    }
}

fn root_fs_to_grpc(r: ross_image::RootFs) -> ross_core::RootFs {
    ross_core::RootFs {
        r#type: r.fs_type,
        layers: r.layers,
    }
}

fn history_to_grpc(h: ross_image::ImageHistory) -> ross_core::ImageHistory {
    ross_core::ImageHistory {
        id: h.id,
        created: None,
        created_by: h.created_by,
        tags: h.tags,
        size: h.size,
        comment: h.comment,
    }
}

fn pull_progress_to_grpc(p: ross_image::PullProgress) -> PullImageProgress {
    PullImageProgress {
        id: p.id,
        status: p.status,
        progress: p.progress,
        progress_detail: p
            .current
            .zip(p.total)
            .map(|(current, total)| ross_core::ProgressDetail { current, total }),
        error: p.error.unwrap_or_default(),
    }
}

fn push_progress_to_grpc(p: ross_image::PushProgress) -> PushImageProgress {
    PushImageProgress {
        id: p.id,
        status: p.status,
        progress: p.progress,
        progress_detail: p
            .current
            .zip(p.total)
            .map(|(current, total)| ross_core::ProgressDetail { current, total }),
        error: p.error.unwrap_or_default(),
    }
}

fn build_progress_to_grpc(b: ross_image::BuildProgress) -> BuildImageProgress {
    BuildImageProgress {
        stream: b.stream,
        error: b.error.unwrap_or_default(),
        progress: b.progress,
        aux: b.aux_id.map(|id| ross_core::BuildAux { id }),
    }
}

fn search_result_to_grpc(s: ross_image::SearchResult) -> ross_core::SearchResult {
    ross_core::SearchResult {
        name: s.name,
        description: s.description,
        star_count: s.star_count,
        is_official: s.is_official,
        is_automated: s.is_automated,
    }
}
