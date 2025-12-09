use crate::storage::FileSystemStore;
use crate::{
    BlobChunk, DeleteBlobRequest, DeleteBlobResponse, DeleteImageIndexRequest,
    DeleteImageIndexResponse, DeleteManifestRequest, DeleteManifestResponse, DeleteTagRequest,
    DeleteTagResponse, GarbageCollectRequest, GarbageCollectResponse, GetBlobRequest,
    GetImageIndexRequest, GetImageIndexResponse, GetManifestRequest, GetManifestResponse,
    GetStoreInfoRequest, GetStoreInfoResponse, ImageIndex, ListBlobsRequest, ListBlobsResponse,
    ListManifestsRequest, ListManifestsResponse, ListTagsRequest, ListTagsResponse, PutBlobRequest,
    PutBlobResponse, PutImageIndexRequest, PutImageIndexResponse, PutManifestRequest,
    PutManifestResponse, ResolveTagRequest, ResolveTagResponse, SetTagRequest, SetTagResponse,
    StatBlobRequest, StatBlobResponse, StoreService,
};
use async_stream::try_stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

const CHUNK_SIZE: usize = 64 * 1024;

pub struct StoreServiceImpl {
    store: Arc<FileSystemStore>,
}

impl StoreServiceImpl {
    pub fn new(store: FileSystemStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

#[tonic::async_trait]
impl StoreService for StoreServiceImpl {
    type GetBlobStream = Pin<Box<dyn Stream<Item = Result<BlobChunk, Status>> + Send>>;

    async fn get_blob(
        &self,
        request: Request<GetBlobRequest>,
    ) -> Result<Response<Self::GetBlobStream>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let data = self
            .store
            .get_blob(&digest, req.offset, req.length)
            .await
            .map_err(Status::from)?;

        let offset = req.offset;
        let stream = try_stream! {
            let mut current_offset = offset;
            for chunk in data.chunks(CHUNK_SIZE) {
                yield BlobChunk {
                    data: chunk.to_vec(),
                    offset: current_offset,
                };
                current_offset += chunk.len() as i64;
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn put_blob(
        &self,
        request: Request<Streaming<PutBlobRequest>>,
    ) -> Result<Response<PutBlobResponse>, Status> {
        let mut stream = request.into_inner();

        let mut media_type = String::new();
        let mut expected_digest = None;
        let mut data = Vec::new();

        use tokio_stream::StreamExt;
        while let Some(req) = stream.next().await {
            let req = req?;
            match req.content {
                Some(crate::put_blob_request::Content::Init(init)) => {
                    media_type = init.media_type;
                    expected_digest = init.expected_digest;
                    if init.expected_size > 0 {
                        data.reserve(init.expected_size as usize);
                    }
                }
                Some(crate::put_blob_request::Content::Data(chunk)) => {
                    data.extend_from_slice(&chunk);
                }
                None => {}
            }
        }

        let (digest, size) = self
            .store
            .put_blob(&media_type, &data, expected_digest.as_ref())
            .await
            .map_err(Status::from)?;

        Ok(Response::new(PutBlobResponse {
            digest: Some(digest),
            size,
        }))
    }

    async fn stat_blob(
        &self,
        request: Request<StatBlobRequest>,
    ) -> Result<Response<StatBlobResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let info = self.store.stat_blob(&digest).await.map_err(Status::from)?;

        Ok(Response::new(StatBlobResponse {
            exists: info.is_some(),
            info,
        }))
    }

    async fn delete_blob(
        &self,
        request: Request<DeleteBlobRequest>,
    ) -> Result<Response<DeleteBlobResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let deleted = self
            .store
            .delete_blob(&digest)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(DeleteBlobResponse { deleted }))
    }

    async fn list_blobs(
        &self,
        request: Request<ListBlobsRequest>,
    ) -> Result<Response<ListBlobsResponse>, Status> {
        let req = request.into_inner();
        let filter = if req.media_type_filter.is_empty() {
            None
        } else {
            Some(req.media_type_filter.as_str())
        };

        let blobs = self.store.list_blobs(filter).await.map_err(Status::from)?;

        Ok(Response::new(ListBlobsResponse {
            blobs,
            continuation_token: String::new(),
        }))
    }

    async fn get_manifest(
        &self,
        request: Request<GetManifestRequest>,
    ) -> Result<Response<GetManifestResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let (content, media_type) = self
            .store
            .get_manifest(&digest)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(GetManifestResponse {
            content,
            media_type,
            digest: Some(digest),
        }))
    }

    async fn put_manifest(
        &self,
        request: Request<PutManifestRequest>,
    ) -> Result<Response<PutManifestResponse>, Status> {
        let req = request.into_inner();

        let (digest, size) = self
            .store
            .put_manifest(&req.content, &req.media_type)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(PutManifestResponse {
            digest: Some(digest),
            size,
        }))
    }

    async fn delete_manifest(
        &self,
        request: Request<DeleteManifestRequest>,
    ) -> Result<Response<DeleteManifestResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let deleted = self
            .store
            .delete_manifest(&digest)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(DeleteManifestResponse { deleted }))
    }

    async fn list_manifests(
        &self,
        request: Request<ListManifestsRequest>,
    ) -> Result<Response<ListManifestsResponse>, Status> {
        let req = request.into_inner();
        let filter = if req.media_type_filter.is_empty() {
            None
        } else {
            Some(req.media_type_filter.as_str())
        };

        let manifests = self
            .store
            .list_manifests(filter)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(ListManifestsResponse {
            manifests,
            continuation_token: String::new(),
        }))
    }

    async fn get_image_index(
        &self,
        request: Request<GetImageIndexRequest>,
    ) -> Result<Response<GetImageIndexResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let content = self.store.get_index(&digest).await.map_err(Status::from)?;

        let index: ImageIndex =
            serde_json::from_slice(&content).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(GetImageIndexResponse {
            index: Some(index),
            digest: Some(digest),
        }))
    }

    async fn put_image_index(
        &self,
        request: Request<PutImageIndexRequest>,
    ) -> Result<Response<PutImageIndexResponse>, Status> {
        let req = request.into_inner();
        let index = req
            .index
            .ok_or_else(|| Status::invalid_argument("index required"))?;

        let content = serde_json::to_vec(&index).map_err(|e| Status::internal(e.to_string()))?;

        let (digest, size) = self.store.put_index(&content).await.map_err(Status::from)?;

        Ok(Response::new(PutImageIndexResponse {
            digest: Some(digest),
            size,
        }))
    }

    async fn delete_image_index(
        &self,
        request: Request<DeleteImageIndexRequest>,
    ) -> Result<Response<DeleteImageIndexResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let deleted = self
            .store
            .delete_index(&digest)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(DeleteImageIndexResponse { deleted }))
    }

    async fn resolve_tag(
        &self,
        request: Request<ResolveTagRequest>,
    ) -> Result<Response<ResolveTagResponse>, Status> {
        let req = request.into_inner();

        let (digest, media_type) = self
            .store
            .resolve_tag(&req.repository, &req.tag)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(ResolveTagResponse {
            digest: Some(digest),
            media_type,
        }))
    }

    async fn set_tag(
        &self,
        request: Request<SetTagRequest>,
    ) -> Result<Response<SetTagResponse>, Status> {
        let req = request.into_inner();
        let digest = req
            .digest
            .ok_or_else(|| Status::invalid_argument("digest required"))?;

        let previous = self
            .store
            .set_tag(&req.repository, &req.tag, &digest)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(SetTagResponse {
            previous_digest: previous,
        }))
    }

    async fn delete_tag(
        &self,
        request: Request<DeleteTagRequest>,
    ) -> Result<Response<DeleteTagResponse>, Status> {
        let req = request.into_inner();

        let deleted = self
            .store
            .delete_tag(&req.repository, &req.tag)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(DeleteTagResponse { deleted }))
    }

    async fn list_tags(
        &self,
        request: Request<ListTagsRequest>,
    ) -> Result<Response<ListTagsResponse>, Status> {
        let req = request.into_inner();

        let tags = self
            .store
            .list_tags(&req.repository)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(ListTagsResponse {
            tags,
            continuation_token: String::new(),
        }))
    }

    async fn garbage_collect(
        &self,
        request: Request<GarbageCollectRequest>,
    ) -> Result<Response<GarbageCollectResponse>, Status> {
        let req = request.into_inner();

        let (blobs_removed, manifests_removed, bytes_freed, removed_digests) = self
            .store
            .garbage_collect(req.dry_run, req.delete_untagged)
            .await
            .map_err(Status::from)?;

        Ok(Response::new(GarbageCollectResponse {
            blobs_removed,
            manifests_removed,
            bytes_freed,
            removed_digests,
        }))
    }

    async fn get_store_info(
        &self,
        _request: Request<GetStoreInfoRequest>,
    ) -> Result<Response<GetStoreInfoResponse>, Status> {
        let (total_size, blob_count, manifest_count, tag_count) =
            self.store.get_store_info().await.map_err(Status::from)?;

        Ok(Response::new(GetStoreInfoResponse {
            store_path: self.store.root().to_string_lossy().to_string(),
            total_size,
            blob_count,
            manifest_count,
            tag_count,
        }))
    }
}
